#![no_main]
#![no_std]

#[cfg(not(target_arch = "wasm32"))]
compile_error!("target arch should be wasm32: compile with '--target wasm32-unknown-unknown'");

extern crate alloc;

mod address;
pub mod constants;
mod entry_points;
mod error;
mod helpers;
mod named_keys;
mod utils;

use crate::constants::*;
use crate::error::Error;
use crate::utils::*;
use alloc::{string::String, vec::Vec};
use casper_contract::{
    contract_api::{runtime, storage},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{contracts::NamedKeys, runtime_args, ContractHash, HashAddr, Key, RuntimeArgs};
use helpers::{get_immediate_caller_key, get_self_key};

#[no_mangle]
fn call() {
    let contract_name: String = runtime::get_named_arg(NFT_BRIDGE_CONTRACT_KEY_NAME);
    let contract_hash_key_name = String::from(contract_name.clone());
    let contract_package_hash_key_name = String::from(contract_name.clone() + "_package_hash");

    let contract_owner: Key = runtime::get_named_arg(ARG_CONTRACT_OWNER);

    let named_keys: NamedKeys = named_keys::default(contract_name, contract_owner);

    // We store contract on-chain
    let (contract_hash, _version) = storage::new_locked_contract(
        entry_points::default(),
        Some(named_keys),
        Some(String::from(contract_package_hash_key_name)),
        None,
    );
    runtime::put_key(CONTRACT_OWNER_KEY_NAME, contract_owner);
    runtime::put_key(contract_hash_key_name.as_str(), Key::from(contract_hash));
}

#[no_mangle]
pub extern "C" fn request_bridge_nft() {
    let contract_hash: Key = runtime::get_named_arg(ARG_NFT_CONTRACT_HASH);
    let identifier_mode = get_identifier_mode_from_runtime_args();
    let user = get_immediate_caller_key();
    let token_identifiers = get_token_identifiers_from_runtime_args(&identifier_mode);
    let self_key = get_self_key();
    cep78_transfer_from(
        &contract_hash,
        user,
        self_key,
        identifier_mode,
        token_identifiers,
    );
    //U256::one()
}

#[no_mangle]
pub extern "C" fn transfer_owner() -> Result<(), Error> {
    let new_contract_owner: Key = runtime::get_named_arg(ARG_CONTRACT_OWNER);
    let current_contract_owner = runtime::get_key(CONTRACT_OWNER_KEY_NAME).unwrap_or_revert();
    if new_contract_owner != current_contract_owner {
        runtime::revert(Error::InvalidContractOwner);
    }
    runtime::put_key(CONTRACT_OWNER_KEY_NAME, new_contract_owner);
    Ok(())
}

#[no_mangle]
pub extern "C" fn unlock_nft() -> Result<(), Error> {
    let caller = get_immediate_caller_key();
    let contract_owner = runtime::get_key(CONTRACT_OWNER_KEY_NAME).unwrap();
    if caller != contract_owner {
        runtime::revert(Error::InvalidAccount);
    }

    let contract_hash: Key = runtime::get_named_arg(ARG_NFT_CONTRACT_HASH);
    let identifier_mode = get_identifier_mode_from_runtime_args();
    let token_identifiers =
        get_token_identifiers_from_runtime_args(&identifier_mode);
    let target: Key = runtime::get_named_arg(ARG_TARGET_KEY);
    let self_key = get_self_key();
    cep78_transfer_from(
        &contract_hash,
        self_key,
        target,
        identifier_mode,
        token_identifiers,
    );
    Ok(())
}

fn cep78_transfer_from(
    contract_hash: &Key,
    source: Key,
    target: Key,
    identifier_mode: NFTIdentifierMode,
    token_identifiers: Vec<TokenIdentifier>,
) {
    let contract_hash_addr: HashAddr = contract_hash.into_hash().unwrap_or_revert();
    let contract_hash: ContractHash = ContractHash::new(contract_hash_addr);
    // let self_address = Address::Account(AccountHash::from_formatted_str("account-hash-32b0eaaa6c0d024e2e7efc34a0a8aad7889cdbb87c71f07cb0eb4f515d5696de").unwrap());
    //let self_key = get_key_from_address(&self_address);
    match identifier_mode {
        NFTIdentifierMode::Ordinal => {
            for token_identifier in token_identifiers {
                let _: (String, Key) = runtime::call_contract(
                    contract_hash,
                    TRANSFER_ENTRY_POINT_NAME,
                    runtime_args! {
                        ARG_SOURCE_KEY => source,
                        ARG_TARGET_KEY => target,
                        ARG_TOKEN_ID => token_identifier.get_index().unwrap()
                    },
                );
            }
        }
        NFTIdentifierMode::Hash => {
            for token_identifier in token_identifiers {
                let _: (String, Key) = runtime::call_contract(
                    contract_hash,
                    TRANSFER_ENTRY_POINT_NAME,
                    runtime_args! {
                        ARG_SOURCE_KEY => source,
                        ARG_TARGET_KEY => target,
                        ARG_TOKEN_HASH => token_identifier.get_hash().unwrap()
                    },
                );
            }
        }
    }
}
