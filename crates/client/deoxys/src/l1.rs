//! Contains the necessaries to perform an L1 verification of the state

use std::error::Error;
use std::sync::Arc;
use mp_commitments::StateCommitment;
use mp_felt::{Felt252Wrapper, Felt252WrapperError};
use starknet_api::block::{BlockNumber, BlockHash};
use reqwest::Url;
use serde_json::Value;
use anyhow::Result;
use tokio::sync::mpsc::{Sender, self};
use tokio_tungstenite::connect_async;

const HTTP_OK: u16 = 200;
pub mod starknet_core_address {
    pub const MAINNET: &[u8] = b"c662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    pub const GOERLI_TESTNET: &[u8] = b"de29d060D45901Fb19ED6C6e959EB22d8626708e";
    pub const GOERLI_INTEGRATION: &[u8] = b"d5c325D183C592C94998000C5e0EED9e6655c020";
    pub const SEPOLIA_TESTNET: &[u8] = b"E2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";
    pub const SEPOLIA_INTEGRATION: &[u8] = b"4737c0c1B4D5b1A687B42610DdabEE781152359c";
}

/// Contains the Starknet verified state on L1
#[derive(Debug, Clone, Deserialize)]
pub struct EthereumStateUpdate {
    pub global_root: StateCommitment,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

type StateUpdateCallback = Arc<dyn Fn(EthereumStateUpdate) + Send + Sync>;

#[derive(Clone)]
pub struct EthereumClient {
    http: reqwest::Client,
    url: Url
}

/// Implementation of the Ethereum client to interact with L1
impl EthereumClient {

    /// Create a new EthereumClient instance with the given RPC URL
    pub fn new(url: Url) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::new(),
            url,
        })
    }

    /// Get current RPC URL
    pub fn get_url(&self) -> Url {
        self.url
    }

    /// Get current RPC URL
    pub fn get_wss(&self) -> Result<Url, Box<dyn Error>> {
        let mut wss_url = self.url.clone();
    
        let new_scheme = match wss_url.scheme() {
            "http" => "ws",
            "https" => "wss",
            "ws" | "wss" => return Ok(wss_url),
            _ => return Err("Unsupported URL scheme".into()),
        };
    
        wss_url.set_scheme(new_scheme).unwrap();
        Ok(wss_url)
    }

    /// Call the Ethereum RPC endpoint with the given JSON-RPC payload
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

    /// Subscribes to a specific event from the Starknet core contract
    async fn get_eth_subscribe(&self, topics: Vec<String>) -> Result<String, Felt252WrapperError> {
        let address = starknet_core_address::MAINNET;

        let params = serde_json::json!({
            "address": address,
            "topics": topics
        });

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_subscribe",
            "params": ["logs", params],
            "id": 1
        });

        let response = match self.call_ethereum(payload).await {
            Ok(response) => response,
            Err(_) => return Err(Felt252WrapperError::InvalidCharacter),
        };

        let subscription_id = match response["result"].as_str() {
            Some(id) => id,
            None => return Err(Felt252WrapperError::InvalidCharacter),
        };

        Ok(subscription_id.to_string())
    }
    
    pub async fn listen_and_update_state(wss_url: Url, subscription_id: &str, tx: Sender<EthereumStateUpdate>) -> Result<(), Box<dyn std::error::Error>> {
        let (ws_stream, _) = connect_async(wss_url).await?;
        let mut ws_stream = ws_stream;
    
        while let Some(message) = futures::StreamExt::next(&mut ws_stream).await {
            let message = message?;
    
            if message.is_text() || message.is_binary() {
                let data = message.into_text()?;
                let event = serde_json::from_str::<EthereumStateUpdate>(&data)?;
    
                if event.subscription_id == subscription_id {
                    tx.send(event.clone()).await.unwrap();
                }
            }
        }
    
        Ok(())
    }


    /// Generates a specific eth_call to the Starknet core contract
    async fn get_eth_call(&self, data: &str) -> Result<Felt252Wrapper, Felt252WrapperError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [
                {
                    "to": starknet_core_address::MAINNET,
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

        let hex_str = hex_str.trim_start_matches("0x");

        Felt252Wrapper::from_hex_be(hex_str)
    }

    /// Get the last Starknet state root verified on L1
    pub async fn get_last_state_root(&self) -> Result<StateCommitment> {
        let data = "0x9588eca2";
        let state_commitment = self.get_eth_call(data).await?;
        Ok(state_commitment.into())
    }

    /// Get the last Starknet block number verified on L1
    pub async fn get_last_block_number(&self) -> Result<BlockNumber> {
        let data = "0x35befa5d";
        let block_number_result = self.get_eth_call(data).await;
        let block_number = block_number_result?;

        match u64::try_from(block_number) {
            Ok(val) => Ok(BlockNumber(val)),
            Err(_) => Err(Felt252WrapperError::FromArrayError.into()),
        }
    }
    
    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_block_hash(&self) -> Result<BlockHash> {
        let data = "0x382d83e3";
        
        // Use `?` to propagate the error if `get_generic_call` results in an Err
        let block_hash_result = self.get_eth_call(data).await;
        let block_hash = block_hash_result?;
        
        // Now we have a block hash and can try to convert it
        match Felt252Wrapper::try_from(block_hash) {
            Ok(val) => Ok(BlockHash(val.into())),
            Err(_) => Err(Felt252WrapperError::FromArrayError.into()),
        }
    }

    /// Get the last Starknet state update verified on the L1
    pub async fn get_initial_state(client: &EthereumClient) -> Result<EthereumStateUpdate, ()> {
        let global_root = client.get_last_state_root().await.map_err(|e| {
            log::error!("Failed to get last state root: {}", e);
            ()
        })?;
        
        let block_number = client.get_last_block_number().await.map_err(|e| {
            log::error!("Failed to get last block number: {}", e);
            ()
        })?;
        
        let block_hash = client.get_last_block_hash().await.map_err(|e| {
            log::error!("Failed to get last block hash: {}", e);
            ()
        })?;
        
        Ok(EthereumStateUpdate {
            global_root,
            block_number,
            block_hash,
        })
    }
}

/// Syncronize with the L1 latest state updates
pub async fn sync(l1_url: Url) {
    let (tx, mut rx) = mpsc::channel(32);

    let client = match EthereumClient::new(l1_url) {
        Ok(client) => client,
        Err(e) => {
            log::error!("Failed to create EthereumClient: {}", e);
            return;
        }
    };
    
    // Get and store the latest state
    let initial_state = match EthereumClient::get_initial_state(&client).await {
        Ok(state) => state,
        Err(_) => return,
    };

    tx.send(initial_state.clone()).await.unwrap();

    log::info!("🚀 Subscribed to L1 state verification on block {}", initial_state.block_number);

    // Listen to LogStateUpdate (0x77552641) update and send changes continusly
    let subscription_id = client.get_eth_subscribe(vec!["0x77552641".to_string()]).await.unwrap();
    let wss_url = client.get_wss().unwrap();
    EthereumClient::listen_and_update_state(wss_url, &subscription_id, tx).await.unwrap();

    // Verify the latest state roots and block against L2
    while let Some(event) = rx.recv().await {
       // verify and store
    }
}
