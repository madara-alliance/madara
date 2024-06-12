//! Utility functions for Deoxys.

use std::time::Instant;

use anyhow::{bail, Context};
use ethers::types::{I256, U256};
use rand::seq::SliceRandom;
use rand::thread_rng;
use reqwest::Client;
use serde_json::Value;
use starknet_api::hash::StarkFelt;
use thiserror::Error;

use crate::l1::{L1StateUpdate, LogStateUpdate};

use super::constant::L1_FREE_RPC_URLS;

// static CONFIG: OnceCell<FetchConfig> = OnceCell::new();

// /// this function needs to be called only once at the start of the program
// pub fn set_config(config: &FetchConfig) {
//     CONFIG.set(config.clone()).expect("CONFIG already initialized");
// }

// pub fn chain_id() -> FieldElement {
//     CONFIG.get().expect("CONFIG not initialized").chain_id
// }

// pub fn l1_core_address() -> mp_block::H160 {
//     CONFIG.get().expect("CONFIG not initialized").l1_core_address
// }

// pub fn gateway() -> Url {
//     CONFIG.get().expect("CONFIG not initialized").gateway.clone()
// }

// pub fn feeder_gateway() -> Url {
//     CONFIG.get().expect("CONFIG not initialized").feeder_gateway.clone()
// }

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

#[derive(Error, Debug)]
pub enum RpcError {
    #[error("HTTP request failed for L1 Free RPC check")]
    HttpRequest(#[from] reqwest::Error),
    #[error("No suitable L1 Free RPC Url found")]
    NoSuitableUrl,
}

struct RpcChecker {
    client: Client,
    urls: &'static [&'static str],
    best_url: Option<&'static str>,
    best_latency: u128,
}

impl RpcChecker {
    fn new(urls: &'static [&'static str]) -> Self {
        RpcChecker { client: Client::new(), urls, best_url: None, best_latency: u128::MAX }
    }

    async fn l1_free_rpc_check(&self, url: &str) -> Result<u128, RpcError> {
        let start = Instant::now();
        let response = self.client.get(url).send().await?;
        if response.status().is_success() {
            Ok(start.elapsed().as_millis())
        } else {
            Ok(u128::MAX)
        }
    }

    async fn l1_free_rpc_best(&mut self) -> Result<&'static str, RpcError> {
        log::info!("🕐 Checking for the best available free L1 RPC urls since none has been provided");
        log::info!("⚠️ This should be for testing purposes only. We recommend providing your own L1 RPC endpoint.");
        for &url in self.urls.iter() {
            match self.l1_free_rpc_check(url).await {
                Ok(latency) if latency < self.best_latency => {
                    log::debug!("New best URL found: {} with latency {} ms", url, latency);
                    self.best_latency = latency;
                    self.best_url = Some(url);
                }
                Ok(latency) => {
                    log::debug!(
                        "URL {} has latency {} ms, which is not better than the current best {} ms",
                        url,
                        latency,
                        self.best_latency
                    );
                }
                Err(e) => {
                    log::debug!("Failed to check latency for URL {}: {:?}", url, e);
                }
            }
        }
        
        match self.best_url {
            Some(best_url) => {
                log::info!("🔗 Using best L1 free RPC url found: {}", best_url);
                Ok(best_url)
            },
            None => Err(RpcError::NoSuitableUrl),
        }
    }
}

pub async fn l1_free_rpc_get() -> Result<&'static str, RpcError> {
    let mut rpc_checker = RpcChecker::new(L1_FREE_RPC_URLS);
    rpc_checker.l1_free_rpc_best().await
}
