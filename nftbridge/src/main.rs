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

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::error::Error;
use crate::helpers::*;
use alloc::{
    string::{String, ToString},
    vec::{self, *},
};
use casper_contract::{
    contract_api::{runtime, storage},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{
    contracts::NamedKeys, runtime_args, ContractHash, HashAddr, Key, RuntimeArgs, U256,
};
use helpers::{get_immediate_caller_key, get_self_key};
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct RequestBridge {
    nft_contract_hash: Key,
    identifier_mode: u8,
    request_id: String,
    request_index: U256,
    receiver_address: String,
    token_ids: Vec<u64>,
    token_hashes: Vec<String>,
}

#[no_mangle]
pub extern "C" fn init() {
    if get_key::<Key>(CONTRACT_HASH_KEY_NAME).is_some() {
        runtime::revert(Error::ContractAlreadyInitialized);
    }
    let contract_hash: Key = runtime::get_named_arg(ARG_CONTRACT_HASH);
    set_key(CONTRACT_HASH_KEY_NAME, contract_hash);
    storage::new_dictionary(REQUEST_IDS).unwrap_or_revert_with(Error::FailedToCreateDictionary);
    storage::new_dictionary(WRAPPED_TOKEN).unwrap_or_revert_with(Error::FailedToCreateDictionary);
    storage::new_dictionary(UNLOCK_IDS)
        .unwrap_or_revert_with(Error::FailedToCreateDictionaryUnlockIds);
}

#[no_mangle]
fn call() {
    let contract_name: String = runtime::get_named_arg(NFT_BRIDGE_CONTRACT_KEY_NAME);
    let dev: Key = runtime::get_named_arg(DEV);
    let contract_hash_key_name = String::from(contract_name.clone());
    let contract_package_hash_key_name = String::from(contract_name.clone() + "_package_hash");

    let contract_owner: Key = runtime::get_named_arg(ARG_CONTRACT_OWNER);
    //let fee_token: Key = runtime::get_named_arg(ARG_FEE_TOKEN_HASH);

    let named_keys: NamedKeys = named_keys::default(contract_name, contract_owner, dev, None);

    // We store contract on-chain
    let (contract_hash, _version) = storage::new_locked_contract(
        entry_points::default(),
        Some(named_keys),
        Some(String::from(contract_package_hash_key_name)),
        None,
    );

    //{
    // let test_string = "test_string_haha_123-456";
    // let test_string_key = get_unlock_id_key(test_string);
    // //}
    runtime::put_key(CONTRACT_OWNER_KEY_NAME, contract_owner);
    runtime::put_key(DEV, dev);
    runtime::put_key(contract_hash_key_name.as_str(), Key::from(contract_hash));

    runtime::call_contract::<()>(
        contract_hash,
        INIT_ENTRY_POINT_NAME,
        runtime_args! {
            ARG_CONTRACT_HASH => Key::from(contract_hash)
        },
    );
}

#[no_mangle]
pub extern "C" fn request_bridge_nft() {
    let contract_hash: Key = runtime::get_named_arg(ARG_NFT_CONTRACT_HASH); //Contract hash of NFT request bridge

    let contract_hash_addr: HashAddr = contract_hash.into_hash().unwrap_or_revert();
    let contract_hash_str: ContractHash = ContractHash::new(contract_hash_addr);
    // check as if token is wrapped token => revert
    if get_dictionary_value_from_key::<String>(WRAPPED_TOKEN, &contract_hash_str.to_string())
        .is_some()
    {
        runtime::revert(Error::InvalidWrappedToken);
    }
    let identifier_mode = get_identifier_mode_from_runtime_args();
    let user = get_immediate_caller_key();
    if user.into_account().is_none() {
        runtime::revert(Error::CallerMustBeAccountHash);
    }

    let self_key = get_self_key();
    let request_id: String = runtime::get_named_arg(ARG_REQUEST_ID);
    if request_id.chars().count() != 64 {
        runtime::revert(Error::RequestIdIlledFormat);
    }
    let decoded = hex::decode(&request_id);
    if decoded.is_err() {
        runtime::revert(Error::RequestIdIlledFormat);
    }
    if request_id.len() != 64 || decoded.unwrap().len() != 32 {
        runtime::revert(Error::RequestIdIlledFormat);
    }

    if get_dictionary_value_from_key::<String>(REQUEST_IDS, &request_id).is_some() {
        runtime::revert(Error::RequestIdRepeated);
    }

    let token_identifiers = get_token_identifiers_from_runtime_args(&identifier_mode);
    if token_identifiers.len() > 10 {
        runtime::revert(Error::TooManyTokenIds);
    }

    let mut current_request_index: U256 = get_key(REQUEST_INDEX).unwrap();
    current_request_index = current_request_index.checked_add(U256::one()).unwrap();

    let receiver_address = runtime::get_named_arg(ARG_RECEIVER_ADDRESS);

    let request_bridge_data = RequestBridge {
        nft_contract_hash: contract_hash,
        identifier_mode: identifier_mode as u8,
        request_id: request_id.clone(),
        request_index: current_request_index,
        receiver_address: receiver_address,
        token_ids: match identifier_mode {
            NFTIdentifierMode::Ordinal => get_named_arg_with_user_errors::<Vec<u64>>(
                ARG_TOKEN_IDS,
                Error::MissingTokenID,
                Error::InvalidTokenIdentifier,
            )
            .unwrap_or_revert(),
            NFTIdentifierMode::Hash => Vec::new(),
        },
        token_hashes: match identifier_mode {
            NFTIdentifierMode::Hash => get_named_arg_with_user_errors::<Vec<String>>(
                ARG_TOKEN_HASHES,
                Error::MissingTokenID,
                Error::InvalidTokenIdentifier,
            )
            .unwrap_or_revert(),
            NFTIdentifierMode::Ordinal => Vec::new(),
        },
    };

    let json_request_data = casper_serde_json_wasm::to_string(&request_bridge_data).unwrap();

    write_dictionary_value_from_key(REQUEST_IDS, &request_id, json_request_data);

    set_key(REQUEST_INDEX, current_request_index);

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
pub extern "C" fn set_wrapped_token() -> Result<(), Error> {
    let wrapped_token: Key = runtime::get_named_arg(ARG_WRAPPED_TOKEN);
    let is_wrapped_token: bool = runtime::get_named_arg(ARG_IS_WRAPPED_TOKEN);
    // let dev = runtime::get_key(DEV).unwrap_or_revert();
    // let caller = helpers::get_verified_caller().unwrap_or_revert();
    let caller = helpers::get_verified_caller().unwrap_or_revert();
    let current_dev = helpers::get_stored_value_with_user_errors::<Key>(
        DEV,
        Error::MissingDev,
        Error::InvalidDev,
    );

    if caller != current_dev {
        runtime::revert(Error::InvalidDev);
    }
    write_dictionary_value_from_key(
        WRAPPED_TOKEN,
        &wrapped_token.to_formatted_string(),
        is_wrapped_token,
    );
    Ok(())
}

#[no_mangle]
pub extern "C" fn transfer_owner() -> Result<(), Error> {
    let new_contract_owner: Key = runtime::get_named_arg(ARG_CONTRACT_OWNER);
    // let current_contract_owner = runtime::get_key(CONTRACT_OWNER_KEY_NAME).unwrap_or_revert();
    // let caller = helpers::get_verified_caller().unwrap_or_revert();
    let caller = helpers::get_verified_caller().unwrap_or_revert();
    let current_contract_owner = helpers::get_stored_value_with_user_errors::<Key>(
        CONTRACT_OWNER_KEY_NAME,
        Error::MissingContractOwner,
        Error::InvalidContractOwner,
    );

    if caller != current_contract_owner {
        runtime::revert(Error::InvalidContractOwner);
    }
    set_key(CONTRACT_OWNER_KEY_NAME, new_contract_owner);
    Ok(())
}

#[no_mangle]
pub extern "C" fn transfer_dev() -> Result<(), Error> {
    let new_dev: Key = runtime::get_named_arg(ARG_NEW_DEV);
    //let current_dev = runtime::get_key(DEV).unwrap_or_revert();
    //let caller = get_immediate_caller_key();
    let caller = helpers::get_verified_caller().unwrap_or_revert();
    let current_dev = helpers::get_stored_value_with_user_errors::<Key>(
        DEV,
        Error::MissingDev,
        Error::InvalidDev,
    );

    if caller != current_dev {
        runtime::revert(Error::InvalidDev);
    }
    set_key(DEV, new_dev);
    Ok(())
}

#[no_mangle]
pub extern "C" fn unlock_nft() -> Result<(), Error> {
    //let caller = get_immediate_caller_key();
    //let contract_owner = runtime::get_key(CONTRACT_OWNER_KEY_NAME).unwrap();
    let caller = helpers::get_verified_caller().unwrap_or_revert();
    let contract_owner = helpers::get_stored_value_with_user_errors::<Key>(
        CONTRACT_OWNER_KEY_NAME,
        Error::MissingContractOwner,
        Error::InvalidContractOwner,
    );
    if caller != contract_owner {
        runtime::revert(Error::InvalidAccount);
    }

    let unlock_id: String = runtime::get_named_arg(ARG_UNLOCK_ID);

    //verify unlock id
    let unlock_id_parts: Vec<&str> = unlock_id.split("-").collect();
    if unlock_id_parts.len() != 6 {
        runtime::revert(Error::UnlockIdIllFormatted);
    }

    {
        if unlock_id_parts[0].len() != 66 || !unlock_id_parts[0].starts_with("0x") {
            runtime::revert(Error::UnlockIdIllFormatted);
        }
        let tx_hash_without_prefix = unlock_id_parts[0].replace("0x", "");
        let decoded = hex::decode(&tx_hash_without_prefix);
        if decoded.is_err() {
            runtime::revert(Error::TxHashUnlockIdIllFormatted);
        }
    }

    let unlock_id_key = get_unlock_id_key(&unlock_id);
    if get_dictionary_value_from_key::<bool>(UNLOCK_IDS, &unlock_id_key).is_some() {
        runtime::revert(Error::UnlockIdRepeated);
    }
    write_dictionary_value_from_key(UNLOCK_IDS, &unlock_id_key, true);

    let mut origin_contract_address = "hash-".to_string();
    origin_contract_address.push_str(unlock_id_parts[4]);
    let contract_hash: Key = runtime::get_named_arg(ARG_NFT_CONTRACT_HASH);
    if contract_hash.to_formatted_string() != origin_contract_address {
        runtime::revert(Error::UnlockIdIllFormatted);
    }

    let identifier_mode = get_identifier_mode_from_runtime_args();
    let token_identifiers = get_token_identifiers_from_runtime_args(&identifier_mode);
    if token_identifiers.len() > 10 {
        runtime::revert(Error::TooManyTokenIds);
    }
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
