//! Governance Voting module.
pub mod consts;
pub mod events;
pub mod voting_state_machine;

use std::collections::BTreeMap;

use casper_dao_utils::{
    casper_contract::unwrap_or_revert::UnwrapOrRevert,
    casper_dao_macros::Instance,
    casper_env::{get_block_time, revert},
    Address,
    Error,
    Mapping,
    VecMapping,
};
use casper_types::U512;

use self::{
    events::{BallotCast, VotingCreatedInfo},
    voting_state_machine::{VotingResult, VotingStateMachine, VotingSummary, VotingType},
};
use super::{
    ballot::Choice,
    ids,
    types::VotingId,
    Ballot,
    BallotCanceled,
    Reason,
    ShortenedBallot,
    VotingCanceled,
    VotingEnded,
};
use crate::{
    refs::{ContractRefs, ContractRefsStorage},
    rules::builder::RulesBuilder,
    voting::validation::rules::can_create_voting::CanCreateVoting,
    Configuration,
    ReputationContractInterface,
    VaNftContractInterface,
};

/// Governance voting is a struct that contracts can use to implement voting. It consists of two phases:
/// 1. Informal voting
/// 2. Formal voting
///
/// Whether formal voting starts depends on informal voting results.
///
/// When formal voting passes, an action can be performed - a contract can be called with voted arguments.
///
/// Governance voting uses [Reputation Token](crate::ReputationContract) to handle reputation staking and
/// [Variable Repo](crate::VariableRepositoryContract) for reading voting configuration.
///
/// For example implementation see [AdminContract](crate::admin::AdminContract)
#[derive(Instance)]
pub struct VotingEngine {
    #[scoped = "contract"]
    refs: ContractRefsStorage,
    voting_states: Mapping<VotingId, Option<VotingStateMachine>>,
    ballots: Mapping<(VotingId, VotingType, Address), Ballot>,
    voters: VecMapping<(VotingId, VotingType), Address>,
}

impl VotingEngine {
    /// Creates new informal [Voting](VotingStateMachine).
    ///
    /// `contract_to_call`, `entry_point` and `runtime_args` parameters define an action that will be performed  when formal voting passes.
    ///
    /// It collects configuration from [Variable Repo](crate::VariableRepositoryContract) and persists it, so they won't change during the voting process.
    ///
    ///
    /// # Events
    /// Emits [`VotingCreated`](VotingCreated), [`BallotCast`](BallotCast)
    ///
    /// # Errors
    /// Throws [`Error::NotEnoughReputation`](Error::NotEnoughReputation) when the creator does not have enough reputation to create a voting
    pub fn create_voting(
        &mut self,
        creator: Address,
        stake: U512,
        configuration: Configuration,
    ) -> VotingCreatedInfo {
        RulesBuilder::new()
            .add_validation(CanCreateVoting::create(
                self.is_va(creator),
                configuration.only_va_can_create(),
            ))
            .validate();

        let should_cast_first_vote = configuration.should_cast_first_vote();

        let voting_ids_address = configuration.voting_ids_address();
        let voting_id = ids::get_next_voting_id(voting_ids_address);
        let mut voting =
            VotingStateMachine::new(voting_id, get_block_time(), creator, configuration);

        let mut used_stake = None;
        if should_cast_first_vote {
            self.cast_vote(
                creator,
                VotingType::Informal,
                Choice::InFavor,
                stake,
                &mut voting,
            );
            used_stake = Some(stake);
        }

        let info = VotingCreatedInfo::new(
            creator,
            voting_id,
            used_stake,
            voting.voting_configuration(),
        );
        self.set_voting(voting);
        info
    }

    /// Finishes voting.
    ///
    /// Depending on type of voting, different actions are performed.
    ///
    /// For informal voting a new formal voting can be created. Reputation staked for this voting is returned to the voters, except for creator. When voting
    /// passes, it is used as a stake for a new voting, otherwise it is burned.
    ///
    /// For formal voting an action will be performed if the result is in favor. Reputation is redistributed to the winning voters. When no quorum is reached,
    /// the reputation is returned, except for the creator - its reputation is then burned.
    ///
    /// # Events
    /// Emits [`VotingEnded`](VotingEnded), [`VotingCreated`](VotingCreated), [`BallotCast`](BallotCast)
    ///
    /// # Errors
    /// Throws [`FinishingCompletedVotingNotAllowed`](Error::FinishingCompletedVotingNotAllowed) if trying to complete already finished voting
    ///
    /// Throws [`FormalVotingTimeNotReached`](Error::FormalVotingTimeNotReached) if formal voting time did not pass
    ///
    /// Throws [`InformalVotingTimeNotReached`](Error::InformalVotingTimeNotReached) if informal voting time did not pass
    ///
    /// Throws [`ArithmeticOverflow`](Error::ArithmeticOverflow) in an unlikely event of a overflow when calculating reputation to redistribute
    pub fn finish_voting(&mut self, voting_id: VotingId, voting_type: VotingType) -> VotingSummary {
        let mut voting = self
            .get_voting(voting_id)
            .unwrap_or_revert_with(Error::VotingDoesNotExist);

        self.assert_voting_type(&voting, voting_type);

        if voting.completed() {
            revert(Error::FinishingCompletedVotingNotAllowed)
        }

        let mut rep_unstakes = BTreeMap::new();
        let mut rep_burns = BTreeMap::new();
        let mut rep_mints = BTreeMap::new();

        let summary = match voting.voting_type() {
            VotingType::Informal => {
                let informal_without_stake = voting.is_informal_without_stake();
                let voting_result = self.finish_informal_voting(&mut voting);
                if !informal_without_stake {
                    let yes_unstakes = self.return_yes_voters_rep(voting_id, VotingType::Informal);
                    let no_unstakes = self.return_no_voters_rep(voting_id, VotingType::Informal);
                    add_to_map(&mut rep_unstakes, Reason::InformalFinished, yes_unstakes);
                    add_to_map(&mut rep_unstakes, Reason::InformalFinished, no_unstakes);
                }

                match voting_result.result() {
                    VotingResult::InFavor | VotingResult::Against => {
                        // It emits BallotCast event, so no need to capture it in VotingEnded event.
                        self.recast_creators_ballot_from_informal_to_formal(&mut voting);
                    }
                    VotingResult::QuorumNotReached => {}
                    VotingResult::Canceled => revert(Error::VotingAlreadyCanceled),
                }
                voting_result
            }
            VotingType::Formal => {
                let voting_result = self.finish_formal_voting(&mut voting);
                match voting_result.result() {
                    VotingResult::InFavor => {
                        let yes_unstakes =
                            self.return_yes_voters_rep(voting_id, VotingType::Formal);
                        let (mints, burns) = self
                            .redistribute_reputation_of_no_voters(voting_id, VotingType::Formal);
                        add_to_map(&mut rep_unstakes, Reason::FormalFinished, yes_unstakes);
                        add_to_map(&mut rep_mints, Reason::FormalWon, mints);
                        add_to_map(&mut rep_burns, Reason::FormalLost, burns);
                    }
                    VotingResult::Against => {
                        let no_unstakes = self.return_no_voters_rep(voting_id, VotingType::Formal);
                        let (mints, burns) = self
                            .redistribute_reputation_of_yes_voters(voting_id, VotingType::Formal);
                        add_to_map(&mut rep_unstakes, Reason::FormalFinished, no_unstakes);
                        add_to_map(&mut rep_mints, Reason::FormalWon, mints);
                        add_to_map(&mut rep_burns, Reason::FormalLost, burns);
                    }
                    VotingResult::QuorumNotReached => {
                        let yes_unstakes =
                            self.return_yes_voters_rep(voting_id, VotingType::Formal);
                        let no_unstakes = self.return_no_voters_rep(voting_id, VotingType::Formal);
                        add_to_map(&mut rep_unstakes, Reason::FormalFinished, yes_unstakes);
                        add_to_map(&mut rep_unstakes, Reason::FormalFinished, no_unstakes);
                    }
                    VotingResult::Canceled => revert(Error::VotingAlreadyCanceled),
                }
                voting_result
            }
        };

        let stats = match summary.voting_type() {
            VotingType::Informal => voting.informal_stats(),
            VotingType::Formal => voting.formal_stats(),
        };

        // Emit VotingEnded event.
        let voting_ended_event = VotingEnded::new(
            voting_id,
            summary.voting_type(),
            summary.result(),
            stats,
            rep_unstakes,
            BTreeMap::new(),
            rep_burns,
            rep_mints,
        );
        voting_ended_event.emit();

        self.set_voting(voting);
        summary
    }

    pub fn finish_voting_without_token_redistribution(
        &mut self,
        voting_id: VotingId,
    ) -> VotingSummary {
        let mut voting = self
            .get_voting(voting_id)
            .unwrap_or_revert_with(Error::VotingDoesNotExist);

        if voting.completed() {
            revert(Error::FinishingCompletedVotingNotAllowed)
        }

        let summary = match voting.voting_type() {
            VotingType::Informal => self.finish_informal_voting(&mut voting),
            VotingType::Formal => self.finish_formal_voting(&mut voting),
        };

        self.set_voting(voting);

        summary
    }

    pub fn summary(&self, voting_id: VotingId) -> VotingSummary {
        let voting = self.get_voting_or_revert(voting_id);
        let voters_len = self.voters.len((voting.voting_id(), voting.voting_type()));
        VotingSummary::new(
            voting.get_result(voters_len),
            voting.voting_type(),
            voting_id,
        )
    }

    fn finish_informal_voting(&mut self, voting: &mut VotingStateMachine) -> VotingSummary {
        if !voting.is_in_time(get_block_time()) {
            revert(Error::InformalVotingTimeNotReached)
        }

        let voting_id = voting.voting_id();
        let voters_len = self.voters.len((voting_id, voting.voting_type()));
        let voting_result = voting.get_result(voters_len);
        match voting_result {
            VotingResult::InFavor | VotingResult::Against => {
                voting.complete_informal_voting();
            }
            VotingResult::QuorumNotReached => {
                voting.finish();
            }
            VotingResult::Canceled => revert(Error::VotingAlreadyCanceled),
        };

        VotingSummary::new(voting_result, VotingType::Informal, voting_id)
    }

    fn finish_formal_voting(&mut self, voting: &mut VotingStateMachine) -> VotingSummary {
        voting.guard_finish_formal_voting(get_block_time());
        let voting_id = voting.voting_id();
        let voters_len = self.voters.len((voting_id, VotingType::Formal));
        let voting_result = voting.get_result(voters_len);

        if voting_result == VotingResult::InFavor {
            self.perform_action(voting.voting_id());
        }

        voting.finish();

        VotingSummary::new(voting_result, VotingType::Formal, voting_id)
    }

    /// Casts a vote
    ///
    /// # Events
    /// Emits [`BallotCast`](BallotCast)
    ///
    /// # Errors
    /// Throws [`VoteOnCompletedVotingNotAllowed`](Error::VoteOnCompletedVotingNotAllowed) if voting is completed
    ///
    /// Throws [`CannotVoteTwice`](Error::CannotVoteTwice) if voter already voted
    pub fn vote(
        &mut self,
        voter: Address,
        voting_id: VotingId,
        voting_type: VotingType,
        choice: Choice,
        stake: U512,
    ) {
        let mut voting = self.get_voting_or_revert(voting_id);
        self.cast_vote(voter, voting_type, choice, stake, &mut voting);
        self.set_voting(voting);
    }

    fn cast_vote(
        &mut self,
        voter: Address,
        voting_type: VotingType,
        choice: Choice,
        stake: U512,
        voting: &mut VotingStateMachine,
    ) {
        let voting_id = voting.voting_id();
        self.assert_voting_type(&voting, voting_type);
        voting.guard_vote(get_block_time());
        self.assert_vote_doesnt_exist(voting_id, voting.voting_type(), voter);
        self.cast_ballot(voter, voting_id, choice, stake, false, voting);
    }

    fn assert_vote_doesnt_exist(
        &mut self,
        voting_id: VotingId,
        voting_type: VotingType,
        voter: Address,
    ) {
        let vote = self.ballots.get(&(voting_id, voting_type, voter));

        if vote.is_some() {
            revert(Error::CannotVoteTwice)
        }
    }

    fn assert_voting_type(&self, voting: &VotingStateMachine, voting_type: VotingType) {
        if voting.voting_type() != voting_type {
            revert(Error::VotingWithGivenTypeNotInProgress)
        }
    }

    // TODO: REFACTOR EVERYTHING
    pub fn cast_ballot(
        &mut self,
        voter: Address,
        voting_id: VotingId,
        choice: Choice,
        stake: U512,
        unbounded: bool,
        voting: &mut VotingStateMachine,
    ) {
        let ballot = Ballot::new(
            voter,
            voting_id,
            voting.voting_type(),
            choice,
            stake,
            unbounded,
            false,
        );

        if !unbounded && !voting.is_informal_without_stake() {
            // Stake the reputation
            self.refs
                .reputation_token()
                .stake_voting(voting_id, ballot.clone().into());
        }

        BallotCast::new(&ballot).emit();

        // Add a voter to the list
        self.voters.add((voting_id, voting.voting_type()), voter);

        // Update the votes list
        self.ballots
            .set(&(voting_id, voting.voting_type(), voter), ballot);

        // update voting
        if unbounded {
            voting.add_unbounded_stake(stake, choice)
        } else {
            voting.add_stake(stake, choice);
        }
    }

    pub fn all_voters(&self, voting_id: VotingId, voting_type: VotingType) -> Vec<Address> {
        self.voters.get_all((voting_id, voting_type))
    }

    /// Returns the [Ballot](Ballot) of voter with `address` and cast on `voting_id`
    pub fn get_ballot(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
        address: Address,
    ) -> Option<Ballot> {
        self.ballots.get_or_none(&(voting_id, voting_type, address))
    }

    /// Returns the nth [Ballot](Ballot) of cast on `voting_id`
    pub fn get_ballot_at(&self, voting_id: VotingId, voting_type: VotingType, i: u32) -> Ballot {
        let address = self
            .get_voter(voting_id, voting_type, i)
            .unwrap_or_revert_with(Error::VoterDoesNotExist);
        self.get_ballot(voting_id, voting_type, address)
            .unwrap_or_revert_with(Error::BallotDoesNotExist)
    }

    /// Returns the address of nth voter who voted on Voting with `voting_id`
    pub fn get_voter(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
        at: u32,
    ) -> Option<Address> {
        self.voters.get_or_none((voting_id, voting_type), at)
    }

    /// Returns the [Voting](VotingStateMachine) for given id
    pub fn get_voting(&self, voting_id: VotingId) -> Option<VotingStateMachine> {
        self.voting_states
            .get_or_none(&voting_id)
            .map(|x| x.unwrap_or_revert())
    }

    pub fn get_voting_or_revert(&self, voting_id: VotingId) -> VotingStateMachine {
        self.get_voting(voting_id)
            .unwrap_or_revert_with(Error::VotingDoesNotExist)
    }

    pub fn set_voting(&self, voting: VotingStateMachine) {
        self.voting_states.set(&voting.voting_id(), Some(voting))
    }

    fn perform_action(&self, voting_id: VotingId) {
        let voting = self.get_voting(voting_id).unwrap_or_revert();
        for contract_call in voting.contract_calls() {
            contract_call.call();
        }
    }

    pub fn unstake_all_reputation(
        &mut self,
        voting_id: VotingId,
        voting_type: VotingType,
    ) -> BTreeMap<Address, U512> {
        let mut transfers = BTreeMap::new();
        let mut ballots = Vec::<ShortenedBallot>::new();
        for i in 0..self.voters.len((voting_id, voting_type)) {
            let ballot = self.get_ballot_at(voting_id, voting_type, i);
            if ballot.unbounded || ballot.canceled {
                continue;
            }
            transfers.insert(ballot.voter, ballot.stake);
            ballots.push(ballot.into());
        }
        self.refs
            .reputation_token()
            .bulk_unstake_voting(voting_id, ballots);
        transfers
    }

    pub fn recast_creators_ballot_from_informal_to_formal(
        &mut self,
        voting: &mut VotingStateMachine,
    ) {
        let voting_id = voting.voting_id();
        let creator = voting.creator();
        let creator_ballot = self
            .get_ballot(voting_id, VotingType::Informal, *creator)
            .unwrap_or_revert_with(Error::BallotDoesNotExist);

        self.cast_ballot(
            *creator,
            voting_id,
            Choice::InFavor,
            creator_ballot.stake,
            creator_ballot.unbounded,
            voting,
        );
    }

    pub fn return_yes_voters_rep(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
    ) -> BTreeMap<Address, U512> {
        let mut summary = BTreeMap::new();
        let mut ballots = Vec::<ShortenedBallot>::new();
        for i in 0..self.voters.len((voting_id, voting_type)) {
            let ballot = self.get_ballot_at(voting_id, voting_type, i);
            if ballot.choice.is_in_favor() && !ballot.unbounded {
                ballots.push(ballot.clone().into());
                summary.insert(ballot.voter, ballot.stake);
            }
        }
        self.refs
            .reputation_token()
            .bulk_unstake_voting(voting_id, ballots);
        summary
    }

    pub fn return_no_voters_rep(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
    ) -> BTreeMap<Address, U512> {
        let mut summary = BTreeMap::new();
        let mut ballots = Vec::<ShortenedBallot>::new();
        for i in 0..self.voters.len((voting_id, voting_type)) {
            let ballot = self.get_ballot_at(voting_id, voting_type, i);
            if ballot.choice.is_against() && !ballot.unbounded {
                ballots.push(ballot.clone().into());
                summary.insert(ballot.voter, ballot.stake);
            }
        }
        self.refs
            .reputation_token()
            .bulk_unstake_voting(voting_id, ballots);
        summary
    }

    pub fn redistribute_reputation_of_no_voters(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
    ) -> (BTreeMap<Address, U512>, BTreeMap<Address, U512>) {
        let voting = self.get_voting_or_revert(voting_id);
        let total_stake_in_favor = voting.stake_in_favor();
        let total_stake_against = voting.stake_against();
        let mut burns: BTreeMap<Address, U512> = BTreeMap::new();
        let mut mints: BTreeMap<Address, U512> = BTreeMap::new();
        let mut ballots: Vec<ShortenedBallot> = Vec::new();

        for i in 0..self.voters.len((voting_id, voting_type)) {
            let ballot = self.get_ballot_at(voting_id, voting_type, i);
            if ballot.unbounded {
                continue;
            }
            if ballot.choice.is_against() {
                ballots.push(ballot.clone().into());
                burns.insert(ballot.voter, ballot.stake);
            } else {
                let amount_to_mint = total_stake_against * ballot.stake / total_stake_in_favor;
                mints.insert(ballot.voter, amount_to_mint);
            }
        }
        self.refs
            .reputation_token()
            .bulk_unstake_voting(voting_id, ballots);
        self.refs
            .reputation_token()
            .bulk_mint_burn(mints.clone(), burns.clone());
        (mints, burns)
    }

    pub fn redistribute_reputation_of_yes_voters(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
    ) -> (BTreeMap<Address, U512>, BTreeMap<Address, U512>) {
        let voting = self.get_voting_or_revert(voting_id);
        let total_stake_in_favor = voting.stake_in_favor();
        let total_stake_against = voting.stake_against();
        let mut burns: BTreeMap<Address, U512> = BTreeMap::new();
        let mut mints: BTreeMap<Address, U512> = BTreeMap::new();
        let mut ballots: Vec<ShortenedBallot> = Vec::new();
        for i in 0..self.voters.len((voting_id, voting_type)) {
            let ballot = self.get_ballot_at(voting_id, voting_type, i);
            if ballot.unbounded {
                continue;
            }
            if ballot.choice.is_in_favor() {
                ballots.push(ballot.clone().into());
                burns.insert(ballot.voter, ballot.stake);
            } else {
                let amount_to_mint = total_stake_in_favor * ballot.stake / total_stake_against;
                mints.insert(ballot.voter, amount_to_mint);
            }
        }
        self.refs
            .reputation_token()
            .bulk_unstake_voting(voting_id, ballots);
        self.refs
            .reputation_token()
            .bulk_mint_burn(mints.clone(), burns.clone());
        (mints, burns)
    }

    fn is_va(&self, address: Address) -> bool {
        !self.refs.va_token().balance_of(address).is_zero()
    }

    /// Get a reference to the governance voting's voters.
    pub fn voters(&self) -> &VecMapping<(VotingId, VotingType), Address> {
        &self.voters
    }

    pub fn voters_count(&self, voting_id: VotingId, voting_type: VotingType) -> u32 {
        self.voters().len((voting_id, voting_type))
    }

    pub fn bound_ballot(&mut self, voting_id: VotingId, worker: Address, voting_type: VotingType) {
        let mut ballot = self
            .get_ballot(voting_id, voting_type, worker)
            .unwrap_or_revert_with(Error::BallotDoesNotExist);

        let mut voting = self.get_voting_or_revert(voting_id);
        voting.bound_stake(ballot.stake, ballot.choice);
        self.set_voting(voting);

        self.refs.reputation_token().mint(worker, ballot.stake);
        self.refs
            .reputation_token()
            .stake_voting(voting_id, ballot.clone().into());

        ballot.unbounded = false;
        self.ballots.set(&(voting_id, voting_type, worker), ballot);
    }

    pub fn voting_exists(&self, voting_id: VotingId, voting_type: VotingType) -> bool {
        let voting = self.get_voting(voting_id);
        match voting {
            None => false,
            Some(voting) => voting.voting_type() == voting_type,
        }
    }

    pub fn slash_voter(&mut self, voter: Address, voting_id: VotingId) {
        let voting = self.get_voting_or_revert(voting_id);
        if &voter == voting.creator() {
            self.cancel_voting(voting);
        } else {
            self.cancel_ballot(voting, voter);
        }
    }

    fn cancel_voting(&mut self, mut voting: VotingStateMachine) {
        let voting_id = voting.voting_id();
        let voting_type = voting.voting_type();
        let unstakes = self.unstake_all_reputation(voting_id, voting_type);
        voting.cancel();
        self.set_voting(voting);

        // Emit event.
        VotingCanceled::new(voting_id, voting_type, unstakes).emit();
    }

    // Note: it doesn't remove a voter from self.votings to keep the quorum num right.
    fn cancel_ballot(&mut self, mut voting: VotingStateMachine, voter: Address) {
        let voting_id = voting.voting_id();
        let ballots_key = (voting_id, voting.voting_type(), voter);
        let mut ballot = self
            .ballots
            .get(&ballots_key)
            .unwrap_or_revert_with(Error::BallotDoesNotExist);

        // Unstake reputation.
        self.refs
            .reputation_token()
            .unstake_voting(voting_id, ballot.clone().into());

        // Update voting.
        let stake = ballot.stake;
        let choice = ballot.choice;
        if ballot.unbounded {
            voting.remove_unbounded_stake(stake, choice)
        } else {
            voting.remove_stake(stake, choice);
        }
        self.set_voting(voting);

        // Emit event.
        BallotCanceled::new(&ballot).emit();

        // Update ballot.
        ballot.canceled = true;
        self.ballots.set(&ballots_key, ballot);
    }
}

fn add_to_map(
    target: &mut BTreeMap<(Address, Reason), U512>,
    reason: Reason,
    source: BTreeMap<Address, U512>,
) {
    for (addr, amount) in source {
        target.insert((addr, reason), amount);
    }
}
