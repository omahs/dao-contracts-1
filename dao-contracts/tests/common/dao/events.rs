use casper_dao_modules::events::{AddedToWhitelist, OwnerChanged, RemovedFromWhitelist};
use casper_dao_utils::TestContract;

use crate::{
    common::{
        params::{events::Event, Contract},
        DaoWorld,
    },
    on_contract,
};

#[allow(dead_code)]
impl DaoWorld {
    pub fn assert_event(&self, contract: &Contract, idx: i32, ev: Event) {
        match ev {
            Event::OwnerChanged(account) => {
                let new_owner = self.get_address(&account);
                self.assert_dao_event(contract, idx, OwnerChanged { new_owner })
            }
            Event::AddedToWhitelist(account) => {
                let address = self.get_address(&account);
                self.assert_dao_event(contract, idx, AddedToWhitelist { address })
            }
            Event::RemovedFromWhitelist(account) => {
                let address = self.get_address(&account);
                self.assert_dao_event(contract, idx, RemovedFromWhitelist { address })
            }

            Event::NftTransfer(from, to, token_id) => {
                let from = from.map(|account| self.get_address(&account));
                let to = to.map(|account| self.get_address(&account));
                let token_id = token_id.0;

                self.assert_dao_event(
                    contract,
                    idx,
                    casper_dao_erc721::events::Transfer { from, to, token_id },
                )
            }
            Event::NftApproval(owner, approved, token_id) => {
                let owner = owner.map(|account| self.get_address(&account));
                let approved = approved.map(|account| self.get_address(&account));
                let token_id = token_id.0;

                self.assert_dao_event(
                    contract,
                    idx,
                    casper_dao_erc721::events::Approval {
                        owner,
                        approved,
                        token_id,
                    },
                )
            }
            // Event::VotingContractCreated(variable_repo, reputation_token, kyc_voter) => self
            //     .assert_dao_event(
            //         contract,
            //         idx,
            //         VotingContractCreated {
            //             voter_contract: self.get_contract_address(&kyc_voter),
            //             variable_repo: self.get_contract_address(&variable_repo),
            //             reputation_token: self.get_contract_address(&reputation_token),
            //         },
            //     ),
            // Event::VotingCreated(
            //     creator,
            //     voting_id,
            //     informal_voting_id,
            //     formal_voting_id,
            //     config_formal_voting_quorum,
            //     config_formal_voting_time,
            //     config_informal_voting_quorum,
            //     config_informal_voting_time,
            // ) => self.assert_dao_event(
            //     contract,
            //     idx,
            //     VotingCreated {
            //         creator: self.get_address(&creator),
            //         voting_id,
            //         informal_voting_id,
            //         formal_voting_id,
            //         config_formal_voting_quorum,
            //         config_formal_voting_time,
            //         config_informal_voting_quorum,
            //         config_informal_voting_time,
            //     },
            // ),
            // Event::BallotCast(voter, voting_id, choice, stake) => self.assert_dao_event(
            //     contract,
            //     idx,
            //     BallotCast {
            //         voter: self.get_address(&voter),
            //         voting_id,
            //         choice: choice.into(),
            //         stake: *stake,
            //     },
            // ),

            // TODO: Reenable those tests.
            _ => {}
        };
    }

    fn assert_dao_event<T>(&self, contract: &Contract, idx: i32, ev: T)
    where
        T: casper_types::bytesrepr::FromBytes + std::cmp::PartialEq + std::fmt::Debug,
    {
        on_contract!(self, contract, assert_event_at(idx, ev));
    }
}
