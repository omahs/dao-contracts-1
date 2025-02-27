use std::{fmt::Debug, str::FromStr};

use cucumber::Parameter;

use super::{Account, Balance, Contract};
use crate::common::helpers;

#[derive(Debug, Clone)]
pub struct Voting {
    pub contract: Contract,
    stake: Balance,
    raw_args: Vec<String>,
}

impl Voting {
    pub fn get_parsed_arg<T>(&self, n: usize) -> T
    where
        T: FromStr,
        <T as FromStr>::Err: Debug,
    {
        helpers::parse::<T>(self.raw_args.get(n), "Couldn't parse voting arg")
    }

    pub fn get_parsed_arg_or_none<T>(&self, n: usize) -> Option<T>
    where
        T: FromStr,
        <T as FromStr>::Err: Debug,
    {
        helpers::parse_or_none::<T>(self.raw_args.get(n))
    }

    pub fn get_stake(&self) -> Balance {
        self.stake
    }
}

impl From<&Vec<String>> for Voting {
    fn from(value: &Vec<String>) -> Self {
        let contract = value.get(0).unwrap().parse().unwrap();
        let stake = value.get(1).unwrap().parse().unwrap();
        let raw_args = value.iter().skip(2).map(|str| str.to_owned()).collect();

        Self {
            contract,
            stake,
            raw_args,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Parameter)]
#[param(name = "voting_type", regex = "formal|informal")]
pub enum VotingType {
    Formal,
    #[default]
    Informal,
}

impl FromStr for VotingType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "formal" => Self::Formal,
            "informal" => Self::Informal,
            invalid => return Err(format!("Invalid `VotingType`: {invalid}")),
        })
    }
}

impl From<VotingType> for casper_dao_contracts::voting::voting_state_machine::VotingType {
    fn from(value: VotingType) -> Self {
        match value {
            VotingType::Formal => {
                casper_dao_contracts::voting::voting_state_machine::VotingType::Formal
            }
            VotingType::Informal => {
                casper_dao_contracts::voting::voting_state_machine::VotingType::Informal
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Parameter)]
#[param(name = "choice", regex = "in favor|against|yes|no|Yes|No")]
pub enum Choice {
    InFavor,
    #[default]
    Against,
}

impl FromStr for Choice {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "yes" => Self::InFavor,
            "in favor" => Self::InFavor,
            "against" => Self::Against,
            "no" => Self::Against,
            invalid => return Err(format!("Invalid `Choice`: {invalid}")),
        })
    }
}

impl From<Choice> for casper_dao_contracts::voting::Choice {
    fn from(value: Choice) -> Self {
        match value {
            Choice::InFavor => casper_dao_contracts::voting::Choice::InFavor,
            Choice::Against => casper_dao_contracts::voting::Choice::Against,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ballot {
    pub voter: Account,
    pub stake: Balance,
    pub choice: Choice,
    pub voting_id: u32,
    pub voting_type: VotingType,
}

#[allow(dead_code)]
#[derive(Default)]
pub struct BallotBuilder {
    voting_id: u32,
    voting_type: VotingType,
}

#[allow(dead_code)]
impl BallotBuilder {
    pub fn with_voting_id(mut self, voting_id: u32) -> Self {
        self.voting_id = voting_id;
        self
    }

    pub fn with_voting_type(mut self, voting_type: VotingType) -> Self {
        self.voting_type = voting_type;
        self
    }

    pub fn build(&self, data: &Vec<String>) -> Ballot {
        let mut ballot: Ballot = data.into();
        ballot.voting_id = self.voting_id;
        ballot.voting_type = self.voting_type;
        ballot
    }
}

impl From<&Vec<String>> for Ballot {
    fn from(value: &Vec<String>) -> Self {
        let voter = helpers::parse(value.get(0), "Couldn't parse voter");
        let stake = helpers::parse_or_default(value.get(1));
        let choice = helpers::parse_or_default(value.get(2));

        Self {
            voter,
            stake,
            choice,
            voting_id: Default::default(),
            voting_type: Default::default(),
        }
    }
}
