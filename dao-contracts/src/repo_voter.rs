//! Contains Repo Voter Contract definition and related abstractions.
//!
//! # General
//! A type of Governance Voting that updates values stored in the [`Variable Repository Contract`].
//!
//! # Voting
//! The Voting process is managed by [`VotingEngine`].
//!
//! [`Variable Repository Contract`]: crate::variable_repository::VariableRepositoryContractInterface
//! [VotingEngine]: crate::voting::VotingEngine
use casper_dao_modules::AccessControl;
use casper_dao_utils::{
    casper_dao_macros::{casper_contract_interface, Event, Instance},
    casper_env::caller,
    Address,
    BlockTime,
    ContractCall,
};
use casper_types::{bytesrepr::Bytes, runtime_args, RuntimeArgs, U512};
use delegate::delegate;

use crate::{
    config::ConfigurationBuilder,
    voting::{
        events::VotingCreatedInfo,
        refs::ContractRefsStorage,
        voting_state_machine::{VotingStateMachine, VotingType},
        Ballot,
        Choice,
        VotingEngine,
        VotingId,
    },
};

#[casper_contract_interface]
pub trait RepoVoterContractInterface {
    /// Constructor function.
    ///
    /// # Note
    /// Initializes contract elements:
    /// * Sets up [`ContractRefsStorage`] by writing addresses of [`Variable Repository`](crate::variable_repository::VariableRepositoryContract),
    /// [`Reputation Token`](crate::reputation::ReputationContract), [`VA Token`](crate::va_nft::VaNftContract).
    /// * Sets [`caller`] as the owner of the contract.
    /// * Adds [`caller`] to the whitelist.
    ///
    /// # Events
    /// * [`OwnerChanged`](casper_dao_modules::events::OwnerChanged),
    /// * [`AddedToWhitelist`](casper_dao_modules::events::AddedToWhitelist),
    fn init(&mut self, variable_repository: Address, reputation_token: Address, va_token: Address);
    /// Creates new RepoVoter voting.
    ///
    /// # Arguments
    /// * `variable_repo_to_edit` takes an [Address] of the[Variable Repo](crate::variable_repository::VariableRepositoryContract)
    /// instance that will be updated
    /// * `key`, `value` and `activation_time` are parameters that will be passed to `update_at`
    /// method of the [Variable Repo](crate::variable_repository::VariableRepositoryContract)
    ///
    /// # Events
    /// * [`RepoVotingCreated`]
    fn create_voting(
        &mut self,
        variable_repo_to_edit: Address,
        key: String,
        value: Bytes,
        activation_time: Option<u64>,
        stake: U512,
    );
    /// Casts a vote. [Read more](VotingEngine::vote())
    fn vote(&mut self, voting_id: VotingId, voting_type: VotingType, choice: Choice, stake: U512);
    /// Finishes voting. Depending on type of voting, different actions are performed.
    /// [Read more](VotingEngine::finish_voting())
    fn finish_voting(&mut self, voting_id: VotingId, voting_type: VotingType);
    /// Returns the address of [Variable Repository](crate::variable_repository::VariableRepositoryContract) contract.
    fn variable_repository_address(&self) -> Address;
    /// Returns the address of [Reputation Token](crate::reputation::ReputationContract) contract.
    fn reputation_token_address(&self) -> Address;
    /// Returns [Voting](VotingStateMachine) for given id.
    fn get_voting(&self, voting_id: VotingId) -> Option<VotingStateMachine>;
    /// Returns the Voter's [`Ballot`].
    fn get_ballot(
        &self,
        voting_id: VotingId,
        voting_type: VotingType,
        address: Address,
    ) -> Option<Ballot>;
    /// see [VotingEngine](VotingEngine::get_voter())
    fn get_voter(&self, voting_id: VotingId, voting_type: VotingType, at: u32) -> Option<Address>;
    /// Checks if voting of a given type and id exists.
    fn voting_exists(&self, voting_id: VotingId, voting_type: VotingType) -> bool;
    /// Erases the voter from voting with the given id. [Read more](VotingEngine::slash_voter).
    fn slash_voter(&mut self, voter: Address, voting_id: VotingId);
    /// Changes the ownership of the contract. Transfers ownership to the `owner`.
    /// Only the current owner is permitted to call this method.
    /// [`Read more`](AccessControl::change_ownership())
    fn change_ownership(&mut self, owner: Address);
    /// Adds a new address to the whitelist.
    /// [`Read more`](AccessControl::add_to_whitelist())
    fn add_to_whitelist(&mut self, address: Address);
    /// Remove address from the whitelist.
    /// [`Read more`](AccessControl::remove_from_whitelist())
    fn remove_from_whitelist(&mut self, address: Address);
    /// Checks whether the given address is added to the whitelist.
    /// [`Read more`](AccessControl::is_whitelisted()).
    fn is_whitelisted(&self, address: Address) -> bool;
    /// Returns the address of the current owner.
    /// [`Read more`](AccessControl::get_owner()).
    fn get_owner(&self) -> Option<Address>;
}

/// RepoVoterContract
///
/// It is responsible for managing variables held in [Variable Repo](crate::variable_repository::VariableRepositoryContract).
///
/// Each change to the variable is being voted on, and when the voting passes, a change is made at given time.
#[derive(Instance)]
pub struct RepoVoterContract {
    refs: ContractRefsStorage,
    voting_engine: VotingEngine,
    access_control: AccessControl,
}

impl RepoVoterContractInterface for RepoVoterContract {
    delegate! {
        to self.voting_engine {
            fn voting_exists(&self, voting_id: VotingId, voting_type: VotingType) -> bool;
            fn get_voting(
            &self,
                voting_id: VotingId,
            ) -> Option<VotingStateMachine>;
            fn get_ballot(
                &self,
                voting_id: VotingId,
                voting_type: VotingType,
                address: Address,
            ) -> Option<Ballot>;
            fn get_voter(&self, voting_id: VotingId, voting_type: VotingType, at: u32) -> Option<Address>;
            fn finish_voting(&mut self, voting_id: VotingId, voting_type: VotingType);
        }

        to self.access_control {
            fn change_ownership(&mut self, owner: Address);
            fn add_to_whitelist(&mut self, address: Address);
            fn remove_from_whitelist(&mut self, address: Address);
            fn is_whitelisted(&self, address: Address) -> bool;
            fn get_owner(&self) -> Option<Address>;
        }

        to self.refs {
            fn variable_repository_address(&self) -> Address;
            fn reputation_token_address(&self) -> Address;
        }
    }

    fn init(&mut self, variable_repository: Address, reputation_token: Address, va_token: Address) {
        self.refs
            .init(variable_repository, reputation_token, va_token);
        self.access_control.init(caller());
    }

    fn create_voting(
        &mut self,
        variable_repo_to_edit: Address,
        key: String,
        value: Bytes,
        activation_time: Option<u64>,
        stake: U512,
    ) {
        let voting_configuration = ConfigurationBuilder::new(&self.refs)
            .contract_call(ContractCall {
                address: variable_repo_to_edit,
                entry_point: "update_at".into(),
                runtime_args: runtime_args! {
                    "key" => key.clone(),
                    "value" => value.clone(),
                    "activation_time" => activation_time,
                },
            })
            .build();

        let (info, _) = self
            .voting_engine
            .create_voting(caller(), stake, voting_configuration);

        RepoVotingCreated::new(variable_repo_to_edit, key, value, activation_time, info).emit();
    }

    fn vote(&mut self, voting_id: VotingId, voting_type: VotingType, choice: Choice, stake: U512) {
        self.voting_engine
            .vote(caller(), voting_id, voting_type, choice, stake);
    }

    fn slash_voter(&mut self, voter: Address, voting_id: VotingId) {
        self.access_control.ensure_whitelisted();
        self.voting_engine.slash_voter(voter, voting_id);
    }
}

/// Informs repo voting has been created.
#[derive(Debug, PartialEq, Eq, Event)]
pub struct RepoVotingCreated {
    variable_repo_to_edit: Address,
    key: String,
    value: Bytes,
    activation_time: Option<u64>,
    creator: Address,
    stake: Option<U512>,
    voting_id: VotingId,
    config_informal_quorum: u32,
    config_informal_voting_time: u64,
    config_formal_quorum: u32,
    config_formal_voting_time: u64,
    config_total_onboarded: U512,
    config_double_time_between_votings: bool,
    config_voting_clearness_delta: U512,
    config_time_between_informal_and_formal_voting: BlockTime,
}

impl RepoVotingCreated {
    pub fn new(
        variable_repo_to_edit: Address,
        key: String,
        value: Bytes,
        activation_time: Option<u64>,
        info: VotingCreatedInfo,
    ) -> Self {
        Self {
            variable_repo_to_edit,
            key,
            value,
            activation_time,
            creator: info.creator,
            stake: info.stake,
            voting_id: info.voting_id,
            config_informal_quorum: info.config_informal_quorum,
            config_informal_voting_time: info.config_informal_voting_time,
            config_formal_quorum: info.config_formal_quorum,
            config_formal_voting_time: info.config_formal_voting_time,
            config_total_onboarded: info.config_total_onboarded,
            config_double_time_between_votings: info.config_double_time_between_votings,
            config_voting_clearness_delta: info.config_voting_clearness_delta,
            config_time_between_informal_and_formal_voting: info
                .config_time_between_informal_and_formal_voting,
        }
    }
}
