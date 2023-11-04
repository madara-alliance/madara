//! Contains the necessaries to perform an L1 verification of the state

use std::{sync::Arc, time::Duration};
use mp_commitments::StateCommitment;
use mp_felt::{Felt252Wrapper, Felt252WrapperError};
use starknet_api::block::{BlockNumber, BlockHash};
use tokio::sync::Mutex;
use tokio::time::sleep;
use reqwest::Url;
use serde_json::Value;
use anyhow::Result;

const HTTP_OK: u16 = 200;
const SLEEP_DURATION: Duration = Duration::from_secs(1800);
const TO_ADDRESS: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";

#[derive(Debug, Clone)]
pub struct EthereumStateUpdate {
    pub state_root: StateCommitment,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

type StateUpdateCallback = Arc<dyn Fn(EthereumStateUpdate) + Send + Sync>;

#[derive(Clone)]
pub struct EthereumClient {
    http: reqwest::Client,
    url: Url,
    on_state_update: Option<StateUpdateCallback>,
}

/// Implementation of the Ethereum client to interact with L1
impl EthereumClient {
    pub fn new(url: Url, on_state_update: Option<StateUpdateCallback>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::new(),
            url,
            on_state_update,
        })
    }

    async fn call_ethereum(&self, value: Value) -> Result<Value> {
        let res = self.http.post(self.url.clone()).json(&value).send().await?;

        let status = res.status();
        let code = status.as_u16();
        if code != HTTP_OK {
            return Err(anyhow::anyhow!("HTTP error: {}", code));
        }

        let response: Value = res.json().await?;
        Ok(response["result"].clone())
    }

    async fn get_generic_call(&self, data: &str) -> Result<Felt252Wrapper, Felt252WrapperError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [
                {
                    "to": TO_ADDRESS,
                    "data": data
                },
                "latest"
            ],
            "id": 2
        });
        
        let response = match self.call_ethereum(payload).await {
            Ok(response) => response,
            Err(e) => {
                return Err(Felt252WrapperError::InvalidCharacter)
            }  
        };
        
        let hex_str = match response.as_str() {
            Some(hex) => hex,
            None => return Err(Felt252WrapperError::InvalidCharacter),
        };

        // If the Ethereum node response includes the '0x' prefix, remove it before parsing.
        let hex_str = hex_str.trim_start_matches("0x");

        Felt252Wrapper::from_hex_be(hex_str)
    }

    pub async fn get_last_state_root(&self) -> Result<StateCommitment> {
        let data = "0x9588eca2";
        let state_commitment = self.get_generic_call(data).await?;
        Ok(state_commitment.into())
    }

    pub async fn get_last_block_number(&self) -> Result<BlockNumber> {
        let data = "0x35befa5d";
        let block_number_result = self.get_generic_call(data).await;
        let block_number = block_number_result?;

        // Now we have a block number and can try to convert it
        match u64::try_from(block_number) {
            Ok(val) => Ok(BlockNumber(val)),
            Err(_) => Err(Felt252WrapperError::FromArrayError.into()),
        }
    }
    
    pub async fn get_last_block_hash(&self) -> Result<BlockHash> {
        let data = "0x382d83e3";
        
        // Use `?` to propagate the error if `get_generic_call` results in an Err
        let block_hash_result = self.get_generic_call(data).await;
        let block_hash = block_hash_result?;
        
        // Now we have a block hash and can try to convert it
        match Felt252Wrapper::try_from(block_hash) {
            Ok(val) => Ok(BlockHash(val.into())),
            Err(_) => Err(Felt252WrapperError::FromArrayError.into()),
        }
    }
}

pub async fn sync(l1_url: Url) {
    let on_state_update = Arc::new(|state_update: EthereumStateUpdate| {
        log::info!("State updated: {:?}", state_update);
    });
    
    // Initialize the EthereumClient
    let client = match EthereumClient::new(l1_url, Some(on_state_update)) {
        Ok(client) => client,
        Err(e) => {
            log::error!("Failed to create EthereumClient: {}", e);
            return;
        }
    };
    
    // Get the initial state
    let initial_state = EthereumStateUpdate {
        state_root: match client.get_last_state_root().await {
            Ok(root) => root,
            Err(e) => {
                log::error!("Failed to get last state root: {}", e);
                return;
            }
        },
        block_number: match client.get_last_block_number().await {
            Ok(number) => number,
            Err(e) => {
                log::error!("Failed to get last block number: {}", e);
                return;
            }
        },
        block_hash: match client.get_last_block_hash().await {
            Ok(hash) => hash,
            Err(e) => {
                log::error!("Failed to get last block hash: {}", e);
                return;
            }
        },
    };
    
    log::info!("🚀 Subscribed to L1 state verification on block {}", initial_state.block_number.0);

    // Wrap the initial state in an Arc<Mutex<>> for thread-safe sharing and mutation
    let shared_state = Arc::new(Mutex::new(initial_state));
    
    // Determine the number of worker tasks to spawn
    let num_workers = 4; // Adjust this value as needed
    
    for _ in 0..num_workers {
        // Clone the shared state and client for each worker
        let state = shared_state.clone();
        let client_cloned = client.clone();
        
        // Spawn a new worker task
        tokio::spawn(async move {
            loop {
                let mut state_guard = state.lock().await;
                
                match client_cloned.get_last_block_number().await {
                    Ok(last_block_number) if last_block_number != state_guard.block_number => {
                        let state_root = match client_cloned.get_last_state_root().await {
                            Ok(root) => root,
                            Err(e) => {
                                log::error!("Error fetching last state root: {}", e);
                                continue; // Retry on error
                            }
                        };
                        
                        let block_hash = match client_cloned.get_last_block_hash().await {
                            Ok(hash) => hash,
                            Err(e) => {
                                log::error!("Error fetching last block hash: {}", e);
                                continue; // Retry on error
                            }
                        };
                        
                        let new_state = EthereumStateUpdate {
                            state_root,
                            block_number: last_block_number,
                            block_hash,
                        };
                        
                        log::info!("🚀 New state root detected on L1: {:?}. Update performed", new_state.state_root.0);
                        
                        if let Some(callback) = &client_cloned.on_state_update {
                            callback(new_state.clone());
                        }
                        
                        *state_guard = new_state;
                    },
                    Ok(_) => {} // No new block, do nothing
                    Err(e) => {
                        log::error!("Error fetching last block number: {}", e);
                    }
                };
                
                // Sleep for a while before checking again
                sleep(SLEEP_DURATION).await;
            }
        });
    }
    
    // Keep the main task alive indefinitely
    // You might want to add some termination condition or signal handling here
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}
