//! Utility functions.
use std::{time::Duration, thread::sleep, error::Error};

use rand::seq::SliceRandom;
use rand::thread_rng;
use reqwest::header;
use serde_json::{Value, json};

use crate::l2::L2StateUpdate;

//TODO: secure the auto calls here

/// Returns the block number of the last block (from Substrate).
pub async fn get_last_synced_block(rpc_port: u16) -> u64 {
    let client = reqwest::Client::new();

    let url = format!("http://localhost:{}/", rpc_port);
    let payload = serde_json::to_vec(&serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "chain_getBlock",
        "params": []
    }))
    .unwrap();

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

pub async fn get_state_update_at(rpc_port: u16, block_number: u64) -> Result<L2StateUpdate, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://localhost:{}", rpc_port);

    let payload = serde_json::to_vec(&json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "starknet_getStateUpdate",
        "params": [{ "block_number": block_number }]
    }))?;

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
                    },
                    Err(e) => {
                        eprintln!("Failed to parse response as JSON: {}", e);
                        attempts += 1;
                        sleep(RETRY_DELAY);
                    }
                }
            },
            Err(e) => {
                eprintln!("Request failed: {}, retrying...", e);
                attempts += 1;
                sleep(RETRY_DELAY);
            }
        }
    }

    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        "Maximum retries exceeded",
    )))
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
    let mut formatted_address = if address.starts_with("0x") {
        address.to_string()
    } else {
        format!("0x{}", address)
    };

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