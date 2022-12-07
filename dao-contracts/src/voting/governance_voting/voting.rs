//! Voting struct with logic for governance voting
use casper_dao_utils::{
    casper_dao_macros::{CLTyped, FromBytes, ToBytes},
    Address,
    BlockTime,
    ContractCall,
    Error,
};
use casper_types::U512;

use crate::{
    voting::{ballot::Choice, types::VotingId},
    DaoConfiguration,
    DaoConfigurationTrait,
};

/// Result of a Voting
#[derive(PartialEq, Eq, Clone, CLTyped, FromBytes, ToBytes, Debug)]
pub enum VotingResult {
    InFavor,
    Against,
    QuorumNotReached,
    Canceled
}

/// Type of Voting (Formal or Informal)
#[derive(CLTyped, FromBytes, ToBytes, Debug, Clone, PartialEq)]
pub enum VotingType {
    Informal,
    Formal,
}

/// State of Voting
#[derive(CLTyped, FromBytes, ToBytes, Debug, Clone, PartialEq)]
pub enum VotingState {
    Informal,
    BetweenVotings,
    Formal,
    Finished,
}

/// Finished Voting summary
#[allow(dead_code)]
#[derive(CLTyped, FromBytes, ToBytes, Clone, Debug)]
pub struct VotingSummary {
    result: VotingResult,
    ty: VotingType,
    informal_voting_id: VotingId,
    formal_voting_id: Option<VotingId>,
}

impl VotingSummary {
    pub fn new(
        result: VotingResult,
        ty: VotingType,
        informal_voting_id: VotingId,
        formal_voting_id: Option<VotingId>,
    ) -> Self {
        Self {
            result,
            ty,
            informal_voting_id,
            formal_voting_id,
        }
    }

    pub fn is_voting_process_finished(&self) -> bool {
        match self.ty {
            VotingType::Informal => self.is_rejected(),
            VotingType::Formal => true,
        }
    }

    pub fn is_formal(&self) -> bool {
        self.formal_voting_id().is_some()
    }

    pub fn formal_voting_id(&self) -> Option<VotingId> {
        self.formal_voting_id
    }

    fn is_rejected(&self) -> bool {
        vec![VotingResult::Against, VotingResult::QuorumNotReached].contains(&self.result)
    }

    pub fn result(&self) -> VotingResult {
        self.result.clone()
    }

    /// Get a reference to the voting summary's ty.
    pub fn voting_type(&self) -> &VotingType {
        &self.ty
    }
}

/// Voting struct
#[derive(Debug, Clone, CLTyped, ToBytes, FromBytes)]
pub struct Voting {
    voting_id: VotingId,
    completed: bool,
    stake_in_favor: U512,
    stake_against: U512,
    unbounded_stake_in_favor: U512,
    unbounded_stake_against: U512,
    start_time: u64,
    informal_voting_id: VotingId,
    formal_voting_id: Option<VotingId>,
    creator: Address,
    voting_configuration: DaoConfiguration,
    is_canceled: bool
}

impl Voting {
    /// Creates new Voting with immutable VotingConfiguration
    pub fn new(
        voting_id: VotingId,
        start_time: u64,
        creator: Address,
        voting_configuration: DaoConfiguration,
    ) -> Self {
        Voting {
            voting_id,
            completed: false,
            stake_in_favor: U512::zero(),
            stake_against: U512::zero(),
            unbounded_stake_in_favor: U512::zero(),
            unbounded_stake_against: U512::zero(),
            start_time,
            informal_voting_id: voting_id,
            formal_voting_id: None,
            creator,
            voting_configuration,
            is_canceled: false
        }
    }

    /// Returns the type of voting
    pub fn get_voting_type(&self) -> VotingType {
        if self.voting_id == self.informal_voting_id {
            VotingType::Informal
        } else {
            VotingType::Formal
        }
    }

    pub fn is_informal_without_stake(&self) -> bool {
        !self.voting_configuration().informal_stake_reputation()
            && self.get_voting_type() == VotingType::Informal
    }

    pub fn start_time(&self) -> u64 {
        self.start_time
    }

    /// Creates new formal voting from self, cloning existing VotingConfiguration
    pub fn create_formal_voting(&self, new_voting_id: VotingId) -> Self {
        let mut voting_configuration = self.voting_configuration.clone();
        if self.is_result_close() {
            voting_configuration.time_between_informal_and_formal_voting *= 2;
        }

        Voting {
            voting_id: new_voting_id,
            completed: false,
            stake_in_favor: U512::zero(),
            stake_against: U512::zero(),
            unbounded_stake_in_favor: U512::zero(),
            unbounded_stake_against: U512::zero(),
            start_time: self.start_time,
            informal_voting_id: self.informal_voting_id,
            formal_voting_id: Some(new_voting_id),
            creator: self.creator,
            voting_configuration,
            // TODO: Shoudn't be false.
            is_canceled: self.is_canceled
        }
    }

    pub fn can_be_completed(&self, block_time: u64) -> bool {
        !self.completed && self.state(block_time) == VotingState::Finished
    }

    /// Sets voting as completed, optionally saves id of newly created formal voting
    pub fn complete(&mut self, formal_voting_id: Option<VotingId>) {
        if formal_voting_id.is_some() {
            self.formal_voting_id = formal_voting_id
        }
        self.completed = true;
    }

    /// Returns if voting is still in voting phase
    pub fn is_in_time(&self, block_time: u64) -> bool {
        match self.get_voting_type() {
            VotingType::Informal => {
                let start_time = self.start_time;
                let voting_time = self.voting_configuration.informal_voting_time();
                start_time + voting_time <= block_time
            }
            VotingType::Formal => {
                self.start_time + self.voting_configuration.formal_voting_time() <= block_time
            }
        }
    }

    pub fn is_in_favor(&self) -> bool {
        self.stake_in_favor >= self.stake_against
    }

    /// Depending on the result of the voting, returns the amount of reputation staked on the winning side
    pub fn get_winning_stake(&self) -> U512 {
        match self.is_in_favor() {
            true => self.stake_in_favor,
            false => self.stake_against,
        }
    }

    pub fn is_result_close(&self) -> bool {
        let stake_in_favor = self.stake_in_favor() + self.unbounded_stake_in_favor();
        let stake_against = self.stake_against() + self.unbounded_stake_against();
        let stake_diff = stake_in_favor.abs_diff(stake_against);
        let stake_diff_percent = stake_diff.saturating_mul(U512::from(100)) / self.total_stake();
        stake_diff_percent <= self.voting_configuration.voting_clearness_delta
    }

    pub fn get_quorum(&self) -> u32 {
        match self.get_voting_type() {
            VotingType::Informal => self
                .voting_configuration
                .governance_informal_voting_quorum(),
            VotingType::Formal => self.voting_configuration.governance_formal_voting_quorum(),
        }
    }

    pub fn get_result(&self, voters_number: u32) -> VotingResult {
        if self.get_quorum() > voters_number {
            VotingResult::QuorumNotReached
        } else if self.is_in_favor() {
            VotingResult::InFavor
        } else {
            VotingResult::Against
        }
    }

    pub fn add_stake(&mut self, stake: U512, choice: Choice) {
        // overflow is not possible due to reputation token having U512 as max
        match choice {
            Choice::InFavor => self.stake_in_favor += stake,
            Choice::Against => self.stake_against += stake,
        }
    }

    pub fn add_unbounded_stake(&mut self, stake: U512, choice: Choice) {
        // overflow is not possible due to reputation token having U512 as max
        match choice {
            Choice::InFavor => self.unbounded_stake_in_favor += stake,
            Choice::Against => self.unbounded_stake_against += stake,
        }
    }

    pub fn bound_stake(&mut self, stake: U512, choice: Choice) {
        match choice {
            Choice::InFavor => self.unbounded_stake_in_favor -= stake,
            Choice::Against => self.unbounded_stake_against -= stake,
        };
        self.add_stake(stake, choice);
    }

    pub fn total_stake(&self) -> U512 {
        // overflow is not possible due to reputation token having U512 as max
        self.total_bounded_stake() + self.total_unbounded_stake()
    }

    pub fn total_bounded_stake(&self) -> U512 {
        // overflow is not possible due to reputation token having U512 as max
        self.stake_in_favor + self.stake_against
    }

    pub fn total_unbounded_stake(&self) -> U512 {
        // overflow is not possible due to reputation token having U512 as max
        self.unbounded_stake_in_favor + self.unbounded_stake_against
    }

    /// Get the voting's voting id.
    pub fn voting_id(&self) -> VotingId {
        self.voting_id
    }

    /// Get the voting's completed.
    pub fn completed(&self) -> bool {
        self.completed
    }

    pub fn validate_vote(&self, block_time: u64) -> Result<(), Error> {
        // Is in time?
        if self.state(block_time) == VotingState::BetweenVotings {
            return Err(Error::VotingDuringTimeBetweenVotingsNotAllowed);
        }

        if self.state(block_time) == VotingState::Finished {
            return Err(Error::VoteOnCompletedVotingNotAllowed);
        }

        Ok(())
    }

    /// Get the voting's stake in favor.
    pub fn stake_in_favor(&self) -> U512 {
        self.stake_in_favor
    }

    /// Get the voting's stake against.
    pub fn stake_against(&self) -> U512 {
        self.stake_against
    }

    pub fn unbounded_stake_in_favor(&self) -> U512 {
        self.unbounded_stake_in_favor
    }

    pub fn unbounded_stake_against(&self) -> U512 {
        self.unbounded_stake_against
    }

    /// Get the voting's informal voting id.
    pub fn informal_voting_id(&self) -> VotingId {
        self.informal_voting_id
    }

    /// Get the voting's formal voting id.
    pub fn formal_voting_id(&self) -> Option<VotingId> {
        self.formal_voting_id
    }

    /// Get the voting's formal voting quorum.
    pub fn formal_voting_quorum(&self) -> u32 {
        self.voting_configuration.governance_formal_voting_quorum()
    }

    /// Get the voting's informal voting quorum.
    pub fn informal_voting_quorum(&self) -> u32 {
        self.voting_configuration
            .governance_informal_voting_quorum()
    }

    /// Get the voting's formal voting time.
    pub fn formal_voting_time(&self) -> u64 {
        self.voting_configuration.formal_voting_time()
    }

    /// Get the voting's informal voting time.
    pub fn informal_voting_time(&self) -> u64 {
        self.voting_configuration.informal_voting_time()
    }

    /// Get the voting's contract call reference.
    pub fn contract_calls(&self) -> &Vec<ContractCall> {
        &self.voting_configuration.contract_calls
    }

    /// Get a reference to the voting's voting configuration.
    pub fn voting_configuration(&self) -> &DaoConfiguration {
        &self.voting_configuration
    }

    pub fn creator(&self) -> &Address {
        &self.creator
    }

    pub fn state(&self, block_time: BlockTime) -> VotingState {
        let informal_voting_end = self.start_time + self.informal_voting_time();
        let between_voting_end = informal_voting_end
            + self
                .voting_configuration
                .time_between_informal_and_formal_voting();
        let voting_end = between_voting_end + self.formal_voting_time();

        if block_time <= informal_voting_end {
            VotingState::Informal
        } else if block_time > informal_voting_end && block_time <= between_voting_end {
            VotingState::BetweenVotings
        } else if block_time > between_voting_end && block_time <= voting_end {
            VotingState::Formal
        } else {
            VotingState::Finished
        }
    }

    pub fn cancel(&mut self) {
        self.is_canceled = true;
    }
}

// #[test]
// fn test_voting_serialization() {
//     use casper_types::bytesrepr::FromBytes;
//     use casper_types::bytesrepr::ToBytes;

//     let voting = Voting {
//         voting_id: 1,
//         completed: false,
//         stake_in_favor: U512::zero(),
//         stake_against: U512::zero(),
//         start_time: 123,
//         informal_voting_id: 1,
//         formal_voting_id: None,
//         voting_configuration: VotingConfiguration {
//             formal_voting_quorum: U512::from(2),
//             formal_voting_time: 2,
//             informal_voting_quorum: U512::from(2),
//             informal_voting_time: 2,
//             create_minimum_reputation: U512::from(2),
//             contract_call: None,
//             cast_first_vote: true,
//             cast_minimum_reputation: U512::from(2),
//             only_va_can_create: true,
//         },
//     };

//     let (voting2, _bytes) = Voting::from_bytes(&voting.to_bytes().unwrap()).unwrap();

//     // TODO: rewrite asserts
//     assert_eq!(voting.voting_id(), voting2.voting_id());
//     assert_eq!(voting.informal_voting_id, voting2.informal_voting_id);
//     assert_eq!(voting.formal_voting_id, voting2.formal_voting_id);
//     assert_eq!(
//         voting.voting_configuration.informal_voting_quorum,
//         voting2.voting_configuration.informal_voting_quorum
//     );
//     assert_eq!(
//         voting.voting_configuration.formal_voting_quorum,
//         voting2.voting_configuration.formal_voting_quorum
//     );
//     assert_eq!(voting.stake_against, voting2.stake_against);
//     assert_eq!(voting.stake_in_favor, voting2.stake_in_favor);
//     assert_eq!(voting.completed, voting2.completed);
//     assert_eq!(
//         voting.voting_configuration.formal_voting_time,
//         voting2.voting_configuration.formal_voting_time
//     );
//     assert_eq!(
//         voting.voting_configuration.informal_voting_time,
//         voting2.voting_configuration.informal_voting_time
//     );
//     assert_eq!(
//         voting.voting_configuration().only_va_can_create,
//         voting2.voting_configuration().only_va_can_create
//     );
//     assert_eq!(voting.start_time, voting2.start_time);
// }
