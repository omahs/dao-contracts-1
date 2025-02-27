//! Functions to interact with the host environment.
use std::{collections::BTreeSet, convert::TryInto};

use casper_contract::{
    contract_api::{runtime, storage, system::create_purse},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{
    bytesrepr::{FromBytes, ToBytes},
    contracts::NamedKeys,
    system::CallStackElement,
    ApiError,
    CLTyped,
    ContractPackageHash,
    EntryPoints,
    Key,
    RuntimeArgs,
    URef,
};

use crate::{
    consts::CONTRACT_MAIN_PURSE,
    events::Events,
    Address,
    BlockTime,
    Error,
    Error::KeyValueStorageError,
};

/// Read value from the storage.
pub fn get_key<T: FromBytes + CLTyped>(name: &str) -> Option<T> {
    match runtime::get_key(name) {
        None => None,
        Some(value) => {
            let key = value
                .try_into()
                .unwrap_or_revert_with(Error::KeyValueStorageError);
            let value = storage::read(key)
                .unwrap_or_revert_with(Error::KeyValueStorageError)
                .unwrap_or_revert_with(KeyValueStorageError);
            Some(value)
        }
    }
}

/// Save value to the storage.
pub fn set_key<T: ToBytes + CLTyped>(name: &str, value: T) {
    match runtime::get_key(name) {
        Some(key) => {
            let key_ref = key
                .try_into()
                .unwrap_or_revert_with(Error::KeyValueStorageError);
            storage::write(key_ref, value);
        }
        None => {
            let key = storage::new_uref(value).into();
            runtime::put_key(name, key);
        }
    }
}

/// Returns address based on a [`CallStackElement`].
///
/// For `Session` and `StoredSession` variants it will return account hash, and for `StoredContract`
/// case it will use contract hash as the address.
fn call_stack_element_to_address(call_stack_element: CallStackElement) -> Address {
    match call_stack_element {
        CallStackElement::Session { account_hash } => Address::from(account_hash),
        CallStackElement::StoredSession { account_hash, .. } => {
            // Stored session code acts in account's context, so if stored session
            // wants to interact, caller's address will be used.
            Address::from(account_hash)
        }
        CallStackElement::StoredContract {
            contract_package_hash,
            ..
        } => Address::from(contract_package_hash),
    }
}

fn take_call_stack_elem(n: usize) -> CallStackElement {
    runtime::get_call_stack()
        .into_iter()
        .nth_back(n)
        .unwrap_or_revert_with(Error::VMInternalError)
}

/// Gets the immediate session caller of the current execution.
///
/// This function ensures that only session code can execute this function, and disallows stored
/// session/stored contracts.
pub fn caller() -> Address {
    let second_elem = take_call_stack_elem(1);
    call_stack_element_to_address(second_elem)
}

/// Gets the address of the currently run contract
pub fn self_address() -> Address {
    let first_elem = take_call_stack_elem(0);
    call_stack_element_to_address(first_elem)
}

/// Record event to the contract's storage.
pub fn emit<T: ToBytes>(event: T) {
    Events::default().emit(event);
}

/// Convert any key to hash.
pub fn to_dictionary_key<T: ToBytes>(key: &T) -> String {
    let preimage = key
        .to_bytes()
        .unwrap_or_revert_with(Error::BytesConversionError);
    let bytes = runtime::blake2b(preimage);
    hex::encode(bytes)
}

/// Calls a contract method by Address
pub fn call_contract<T: CLTyped + FromBytes>(
    address: Address,
    entry_point: &str,
    runtime_args: RuntimeArgs,
) -> T {
    let contract_package_hash = address
        .as_contract_package_hash()
        .unwrap_or_revert_with(Error::InvalidAddress);
    runtime::call_versioned_contract(*contract_package_hash, None, entry_point, runtime_args)
}

/// Creates a new contract and initializes it by calling a constructor function.
pub fn install_contract(
    package_hash: &str,
    entry_points: EntryPoints,
    initializer: impl FnOnce(ContractPackageHash),
) {
    // Create a new contract package hash for the contract.
    let (contract_package_hash, owner_token) = storage::create_contract_package_at_hash();
    runtime::put_key(package_hash, contract_package_hash.into());
    runtime::put_key(&format!("{package_hash}_owner_token"), owner_token.into());

    let init_access: URef =
        storage::create_contract_user_group(contract_package_hash, "init", 1, Default::default())
            .unwrap_or_revert_with(Error::VMInternalError)
            .pop()
            .unwrap_or_revert_with(Error::VMInternalError);

    storage::add_contract_version(contract_package_hash, entry_points, NamedKeys::new());

    // Call constructor method.
    initializer(contract_package_hash);

    // Revoke access to init.
    let mut urefs = BTreeSet::new();
    urefs.insert(init_access);
    storage::remove_contract_user_group_urefs(contract_package_hash, "init", urefs)
        .unwrap_or_revert_with(Error::VMInternalError);
}

/// Returns the current [`BlockTime`].
pub fn get_block_time() -> BlockTime {
    u64::from(runtime::get_blocktime())
}

/// Stops execution of a contract and reverts execution effects with a given error.
pub fn revert<T: Into<ApiError>>(error: T) -> ! {
    runtime::revert(error);
}

/// Returns an [`URef`] of the contracts' main purse.
pub fn contract_main_purse() -> URef {
    match runtime::get_key(CONTRACT_MAIN_PURSE) {
        None => {
            let main_purse = create_purse();
            runtime::put_key(CONTRACT_MAIN_PURSE, Key::from(main_purse));
            main_purse
        }
        Some(value) => *value.as_uref().unwrap_or_revert_with(Error::PurseError),
    }
}
