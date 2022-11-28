use std::{fmt::Debug, str::FromStr};

use casper_dao_contracts::voting::voting::VotingType;
use casper_types::{
    bytesrepr::{Bytes, ToBytes},
    U256,
    U512,
};

/// Converts a string value from Gherkin scenario to a `Bytes` representation of the value
pub fn value_to_bytes(value: &str) -> Bytes {
    match value {
        "true" => true.to_bytes().unwrap().into(),
        "false" => false.to_bytes().unwrap().into(),
        _ => {
            let value: f64 = value.parse().unwrap();
            let value = (value * 1000f64) as u64;
            U256::from(value).to_bytes().unwrap().into()
        }
    }
}

pub fn to_rep(v: &str) -> U256 {
    U256::from((v.parse::<f32>().unwrap() * 1_000f32) as u32) * 1_000_000
}

pub fn to_cspr(v: &str) -> U512 {
    U512::from((v.parse::<f32>().unwrap() * 1_000f32) as u32) * 1_000_000
}

pub fn is_cspr_close_enough(a: U512, b: U512) -> bool {
    let diff = if a > b { a - b } else { b - a };
    diff < U512::from(10_000_000)
}

pub fn is_rep_close_enough(a: U256, b: U256) -> bool {
    let diff = if a > b { a - b } else { b - a };
    diff < U256::from(10_000_000)
}

pub fn to_voting_type(value: &str) -> VotingType {
    match value {
        "formal" => VotingType::Formal,
        "informal" => VotingType::Informal,
        _ => panic!("Unexpected voting type {}", value),
    }
}

pub fn parse<T>(item: Option<&String>, err_msg: &str) -> T
where
    T: FromStr,
    <T as FromStr>::Err: Debug,
{
    item.expect(err_msg).parse::<T>().expect("Parsing failed.")
}

pub fn parse_or_default<T: FromStr + Default>(item: Option<&String>) -> T {
    match item {
        Some(value) => value.parse::<T>().unwrap_or_default(),
        None => T::default(),
    }
}

pub fn parse_or_none<T: FromStr>(item: Option<&String>) -> Option<T> {
    match item {
        Some(value) => {
            if value.is_empty() {
                None
            } else {
                value.parse::<T>().ok()
            }
        }
        None => None,
    }
}
