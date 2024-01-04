//! Contains the necessaries to perform an L1 verification of the state

use std::error::Error;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use hex::encode;
use lazy_static::lazy_static;
use mp_commitments::StateCommitment;
use mp_felt::{Felt252Wrapper, Felt252WrapperError};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use starknet_api::block::{BlockHash, BlockNumber};
use tokio::sync::mpsc::{self, Sender};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::l2::STARKNET_STATE_UPDATE;
use crate::utility::{format_address, get_state_update_at};

lazy_static! {
    /// Shared latest L2 state update verified on L1
    pub static ref ETHEREUM_STATE_UPDATE: Arc<Mutex<L1StateUpdate>> = Arc::new(Mutex::new(L1StateUpdate {
        global_root: StateCommitment::default(),
        block_number: BlockNumber::default(),
        block_hash: BlockHash::default(),
    }));
}

pub mod starknet_core_address {
    pub const MAINNET: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    pub const GOERLI_TESTNET: &str = "0xde29d060D45901Fb19ED6C6e959EB22d8626708e";
    pub const GOERLI_INTEGRATION: &str = "0xd5c325D183C592C94998000C5e0EED9e6655c020";
    pub const SEPOLIA_TESTNET: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";
    pub const SEPOLIA_INTEGRATION: &str = "0x4737c0c1B4D5b1A687B42610DdabEE781152359c";
}

/// Contains the Starknet verified state on L1
#[derive(Debug, Clone, Deserialize)]
pub struct L1StateUpdate {
    pub global_root: StateCommitment,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

#[derive(Clone)]
pub struct EthereumClient {
    _http: reqwest::Client,
    url: Url,
}

/// Implementation of the Ethereum client to interact with L1
impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub fn new(url: Url) -> Result<Self> {
        Ok(Self { _http: reqwest::Client::new(), url })
    }

    /// Get current RPC URL
    pub fn get_url(&self) -> Url {
        self.get_wss().unwrap()
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
        let (mut socket, _) =
            connect_async(&self.get_url()).await.map_err(|e| anyhow::anyhow!("WebSocket connect error: {}", e))?;

        let request = serde_json::to_string(&value)?;
        socket.send(Message::Text(request)).await.map_err(|e| anyhow::anyhow!("WebSocket send error: {}", e))?;

        if let Some(message) = socket.next().await {
            let message = message.map_err(|e| anyhow::anyhow!("WebSocket message error: {}", e))?;
            if let Message::Text(text) = message {
                let response: Value = serde_json::from_str(&text)?;
                return Ok(response["result"].clone());
            }
        }

        Err(anyhow::anyhow!("No response received from WebSocket"))
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
            Err(_e) => return Err(Felt252WrapperError::InvalidCharacter),
        };

        Ok(response.to_string())
    }

    pub async fn listen_and_update_state(
        wss_url: Url,
        subscription_id: &str,
        tx: Sender<L1StateUpdate>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (ws_stream, _) = connect_async(wss_url).await?;
        let mut ws_stream = ws_stream;

        while let Some(message) = futures::StreamExt::next(&mut ws_stream).await {
            let message = message?;

            if message.is_text() || message.is_binary() {
                let data = message.into_text()?;
                let event = serde_json::from_str::<L1StateUpdate>(&data)?;
                println!("ethereum: {:?}", data);
                if subscription_id != subscription_id {
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
            Err(_e) => return Err(Felt252WrapperError::InvalidCharacter),
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
    pub async fn get_initial_state(client: &EthereumClient) -> Result<L1StateUpdate, ()> {
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

        Ok(L1StateUpdate { global_root, block_number, block_hash })
    }
}

/// Update the L1 state with the latest data
pub fn update_l1(state_update: L1StateUpdate) {
    log::info!(
        "🔄 Updated L1 head: Number: #{}, Hash: {}, Root: {}",
        state_update.block_number,
        format_address(&state_update.block_hash.to_string()),
        format_address(&encode(state_update.global_root.0.to_bytes_be()))
    );

    {
        let last_state_update = ETHEREUM_STATE_UPDATE.clone();
        let mut new_state_update = last_state_update.lock().unwrap();
        *new_state_update = state_update.clone();
    }
}

/// Verify the L1 state with the latest data
pub async fn verify_l1(state_update: L1StateUpdate, rpc_port: u16) -> Result<(), String> {
    // Minimize the scope of the lock
    let starknet_state_block_number = {
        let starknet_state = STARKNET_STATE_UPDATE.lock().map_err(|e| e.to_string())?;
        starknet_state.block_number
    };

    // Check if the node reached the latest verified state on Ethereum
    if state_update.block_number > starknet_state_block_number {
        return Err("🚨 L1 state verification failed: Node still syncing".into());
    }

    if state_update.block_number <= starknet_state_block_number {
        let current_state_update = get_state_update_at(rpc_port, state_update.block_number.0)
            .await
            .map_err(|e| format!("Error retrieving state update: {}", e))?;

        // Verifying Block Hash and State Root against L2
        match (
            current_state_update.global_root == state_update.global_root,
            current_state_update.block_hash == state_update.block_hash,
        ) {
            (false, _) => Err("🚨 L1 state verification failed: State root does not match".into()),
            (_, false) => Err("🚨 L1 state verification failed: Block hash does not match".into()),
            (true, true) => {
                log::info!(
                    "✅ Verified L2 state via L1: #{}, Hash: {}, Root: {}",
                    state_update.block_number,
                    format_address(&state_update.block_hash.to_string()),
                    format_address(&encode(state_update.global_root.0.to_bytes_be()))
                );
                Ok(())
            }
        }
    } else {
        Ok(())
    }
}

/// Syncronize with the L1 latest state updates
pub async fn sync(l1_url: Url, rpc_port: u16) {
    let (tx, mut rx) = mpsc::channel(32);

    let client = match EthereumClient::new(l1_url) {
        Ok(client) => client,
        Err(e) => {
            log::error!("Failed to create EthereumClient: {}", e);
            return;
        }
    };

    log::info!("🚀 Subscribed to L1 state verification");

    // Get and store the latest state
    let initial_state = match EthereumClient::get_initial_state(&client).await {
        Ok(state) => state,
        Err(_) => return,
    };

    update_l1(initial_state);

    // Listen to LogStateUpdate (0x77552641) update and send changes continusly
    let wss_url = client.get_wss().unwrap();
    let subscription_id = client.get_eth_subscribe(vec!["0x77552641".to_string()]).await.unwrap();
    EthereumClient::listen_and_update_state(wss_url, &subscription_id, tx).await.unwrap();

    // Verify the latest state roots and block against L2
    while let Some(new_state_update) = rx.recv().await {
        let _ = verify_l1(new_state_update.clone(), rpc_port).await;
        update_l1(new_state_update);
    }
}
