//! Utility functions for Deoxys.

use std::sync::RwLock;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context};
use ethers::types::{I256, U256};
use lazy_static::lazy_static;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reqwest::header;
use serde_json::{json, Value};
use starknet_api::hash::StarkFelt;

use crate::fetch::fetchers::FetchConfig;
use crate::l1::{L1StateUpdate, LogStateUpdate};
use crate::l2::L2StateUpdate;

// TODO: find a better place to store this
lazy_static! {
    /// Store the configuration globally, using a RwLock to allow for concurrent reads and exclusive writes
    static ref CONFIG: RwLock<Option<FetchConfig>> = RwLock::new(None);
}

/// this function needs to be called only once at the start of the program
pub fn update_config(config: &FetchConfig) {
    let mut new_config = CONFIG.write().expect("failed to acquire write lock on CONFIG");
    *new_config = Some(config.clone());
}

pub fn get_config() -> Option<FetchConfig> {
    let guard = CONFIG.read().expect("Failed to acquire read lock on CONFIG");
    guard.as_ref().map(Clone::clone)
}

// TODO: secure the auto calls here

pub async fn get_state_update_at(rpc_port: u16, block_number: u64) -> anyhow::Result<L2StateUpdate> {
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

    bail!("Maximum retries exceeded")
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

pub fn u256_to_starkfelt(u256: U256) -> anyhow::Result<StarkFelt> {
    let mut bytes = [0u8; 32];
    u256.to_big_endian(&mut bytes);
    StarkFelt::new(bytes).context("converting U256 to StarkFelt")
}

pub fn convert_log_state_update(log_state_update: LogStateUpdate) -> anyhow::Result<L1StateUpdate> {
    let block_number = if log_state_update.block_number >= I256::zero() {
        log_state_update.block_number.low_u64()
    } else {
        bail!("Block number is negative");
    };

    let global_root = u256_to_starkfelt(log_state_update.global_root)?;
    let block_hash = u256_to_starkfelt(log_state_update.block_hash)?;

    Ok(L1StateUpdate { block_number, global_root, block_hash })
}
