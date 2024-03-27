//! Utility functions for Deoxys.

use std::error::Error;
use std::sync::RwLock;
use std::thread::sleep;
use std::time::Duration;

use bitvec::order::Msb0;
use bitvec::view::AsBits;
use ethers::types::{I256, U256};
use lazy_static::lazy_static;
use mp_types::block::DBlockT;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reqwest::header;
use serde_json::{json, Value};
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_runtime::traits::UniqueSaturatedInto;
use starknet_api::hash::StarkFelt;
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::StateUpdate;

use crate::fetch::fetchers::FetchConfig;
use crate::l1::{L1StateUpdate, LogStateUpdate};
use crate::l2::{L2StateUpdate, STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER};

// TODO: find a better place to store this
lazy_static! {
    /// Store the configuration globally, using a RwLock to allow for concurrent reads and exclusive writes
    static ref CONFIG: RwLock<Option<FetchConfig>> = RwLock::new(None);
}

/// this function needs to be called only once at the start of the program
pub fn update_config(config: &FetchConfig) {
    let mut new_config = CONFIG.write().expect("Failed to acquire write lock on CONFIG");
    *new_config = Some(config.clone());
}

pub fn get_config() -> Result<FetchConfig, &'static str> {
    let config_guard = CONFIG.read().expect("Failed to acquire read lock on CONFIG");
    match &*config_guard {
        Some(config) => Ok(config.clone()),
        None => Err("Configuration not set yet"),
    }
}

// TODO: secure the auto calls here

/// Returns the block number of the last block (from Substrate).
pub async fn get_last_synced_block(rpc_port: u16) -> u64 {
    let client = reqwest::Client::new();

    let url = format!("http://localhost:{}/", rpc_port);
    let request = serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "chain_getBlock",
        "params": []
    });
    let payload = serde_json::to_vec(&request).unwrap();

    let response: serde_json::Value = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let number_as_hex = response["result"]["block"]["header"]["number"].as_str().unwrap();
    u64::from_str_radix(&number_as_hex[2..], 16).unwrap()
}

/// Returns the block hash for a given block number (from Substrate).
pub async fn get_block_hash_by_number(rpc_port: u16, block_number: u64) -> Option<String> {
    let client = reqwest::Client::new();

    let url = format!("http://localhost:{}/", rpc_port);
    let request = serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "chain_getBlockHash",
        "params": [block_number]
    });
    let payload = serde_json::to_vec(&request).unwrap();
    let response: serde_json::Value = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    response["result"].as_str().map(String::from)
}

pub async fn get_state_update_at(rpc_port: u16, block_number: u64) -> Result<L2StateUpdate, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://localhost:{}", rpc_port);

    let request = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "starknet_getStateUpdate",
        "params": [{ "block_number": block_number }]
    });
    let payload = serde_json::to_vec(&request)?;

    const MAX_ATTEMPTS: u8 = 3;
    const RETRY_DELAY: Duration = Duration::from_secs(5);
    let mut attempts = 0;

    while attempts < MAX_ATTEMPTS {
        let response = client
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .body(payload.clone())
            .send()
            .await;

        match response {
            Ok(response) => {
                let json_response = response.json::<Value>().await;
                match json_response {
                    Ok(json_response) => {
                        if let Some(result) = json_response.get("result") {
                            let state_update: L2StateUpdate = serde_json::from_value(result.clone())?;
                            return Ok(state_update);
                        } else {
                            eprintln!("No result found in response");
                            attempts += 1;
                            sleep(RETRY_DELAY);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to parse response as JSON: {}", e);
                        attempts += 1;
                        sleep(RETRY_DELAY);
                    }
                }
            }
            Err(e) => {
                eprintln!("Request failed: {}, retrying...", e);
                attempts += 1;
                sleep(RETRY_DELAY);
            }
        }
    }

    Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Maximum retries exceeded")))
}

/// Returns a random Pokémon name.
pub async fn get_random_pokemon_name() -> Result<String, Box<dyn std::error::Error>> {
    let res = reqwest::get("https://pokeapi.co/api/v2/pokemon/?limit=1000").await?;
    let body = res.text().await?;
    let json: Value = serde_json::from_str(&body)?;

    let pokemon_array = json["results"].as_array().unwrap();
    let mut rng = thread_rng();
    let random_pokemon = pokemon_array.choose(&mut rng).unwrap();

    Ok(random_pokemon["name"].as_str().unwrap().to_string())
}

/// Returns a truncated version of the given address
pub fn format_address(address: &str) -> String {
    let mut formatted_address = if address.starts_with("0x") { address.to_string() } else { format!("0x{}", address) };

    if let Some(non_zero_index) = formatted_address[2..].find(|c: char| c != '0') {
        formatted_address = format!("0x{}", &formatted_address[2 + non_zero_index..]);
    }

    if formatted_address.len() > 10 {
        let start = &formatted_address[0..6];
        let end = &formatted_address[formatted_address.len() - 4..];
        format!("{}...{}", start, end)
    } else {
        formatted_address
    }
}

pub fn u256_to_starkfelt(u256: U256) -> Result<StarkFelt, &'static str> {
    let mut bytes = [0u8; 32];
    u256.to_big_endian(&mut bytes);
    StarkFelt::new(bytes).map_err(|_| "Failed to convert U256 to StarkFelt")
}

pub fn convert_log_state_update(log_state_update: LogStateUpdate) -> Result<L1StateUpdate, &'static str> {
    let block_number = if log_state_update.block_number >= I256::zero() {
        log_state_update.block_number.low_u64()
    } else {
        return Err("Block number is negative");
    };

    let global_root = u256_to_starkfelt(log_state_update.global_root)?;
    let block_hash = u256_to_starkfelt(log_state_update.block_hash)?;

    Ok(L1StateUpdate { block_number, global_root, block_hash })
}

pub fn get_highest_block_hash_and_number() -> (FieldElement, u64) {
    *STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER
        .read()
        .expect("Failed to acquire read lock on STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER")
}

/// Retrieves Deoxys block hash from state update
pub fn block_hash_deoxys(state_update: &StateUpdate) -> FieldElement {
    state_update.block_hash.unwrap()
}

/// Retrieves Substrate block hash from rpc client
pub fn block_hash_substrate<C>(client: &C, block_number: u64) -> Option<H256>
where
    C: HeaderBackend<DBlockT>,
{
    client
        .hash(UniqueSaturatedInto::unique_saturated_into(block_number))
        .unwrap()
        .map(|hash| H256::from_slice(hash.as_bits::<Msb0>().to_bitvec().as_raw_slice()))
}
