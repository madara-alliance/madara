//! Utility functions for Deoxys.

use std::error::Error;
use std::thread::sleep;
use std::time::Duration;

use ethers::types::I256;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reqwest::header;
use serde_json::{json, Value};
use starknet_api::hash::StarkFelt;
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::BlockId;
use starknet_providers::SequencerGatewayProvider;

use crate::l1::{L1StateUpdate, LogStateUpdate};
use crate::l2::{L2StateUpdate, STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER};

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

pub async fn update_highest_block_hash_and_number(client: &SequencerGatewayProvider) -> Result<(), String> {
    let block = client.get_block(BlockId::Latest).await.map_err(|e| format!("failed to get block: {e}"))?;

    let hash = block.block_hash.ok_or("block hash not found")?;
    let number = block.block_number.ok_or("block number not found")?;

    let last_highest_block_hash_and_number = STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER.clone();
    let mut new_highest_block_hash_and_number = last_highest_block_hash_and_number.lock().unwrap();
    *new_highest_block_hash_and_number = (hash, number);

    Ok(())
}

pub fn get_highest_block_hash_and_number() -> (FieldElement, u64) {
    *STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER.lock().expect("failed to lock STARKNET_HIGHEST_BLOCK_HASH_AND_NUMBER")
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

pub fn event_to_l1_state_update(log_state_update: LogStateUpdate) -> Result<L1StateUpdate, &'static str> {
    let block_number_u64 = if log_state_update.block_number >= I256::from(0) {
        log_state_update.block_number.low_u64()
    } else {
        return Err("Block number is negative");
    };

    let global_root_u128 = log_state_update.global_root.low_u128();
    let block_hash_u128 = log_state_update.block_hash.low_u128();

    if global_root_u128 != log_state_update.global_root.low_u128()
        || block_hash_u128 != log_state_update.block_hash.low_u128()
    {
        return Err("Conversion from U256 to u128 resulted in data loss");
    }

    let global_root = StarkFelt::from(global_root_u128);
    let block_hash = StarkFelt::from(block_hash_u128);

    Ok(L1StateUpdate { block_number: block_number_u64, global_root, block_hash })
}
