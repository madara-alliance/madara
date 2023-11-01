//! Contains the necessaries to perform an L1 verification of the state

use std::{sync::Arc, time::Duration};
use mp_felt::{Felt252Wrapper, Felt252WrapperError};
use starknet_api::block::{BlockNumber, BlockHash};
use starknet_api::hash::StarkHash;
use starknet_ff::FieldElement;
use tokio::sync::Mutex;
use tokio::time::sleep;
use reqwest::Url;
use serde_json::Value;
use anyhow::Result;

const HTTP_OK: u16 = 200;
const SLEEP_DURATION: Duration = Duration::from_secs(1800);
const TO_ADDRESS: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";

#[derive(Default, Debug, Clone)]
pub struct StateCommitment(pub Felt252Wrapper);

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
    
        let response = self.call_ethereum(payload).await.unwrap();
        let hex_str = response
            .as_str()
            .ok_or(Felt252WrapperError::InvalidCharacter)?;

        Felt252Wrapper::from_hex_be(hex_str)
    }

    pub async fn get_last_state_root(&self) -> Result<StateCommitment> {
        let data = "0x9588eca2";
        let state_commitment = self.get_generic_call(data).await?;
        Ok(StateCommitment(state_commitment))
    }

    pub async fn get_last_block_number(&self) -> Result<BlockNumber> {
        let data = "0x666bd0be";
        let block_number = self.get_generic_call(data).await;
        match u64::try_from(block_number.unwrap()) {
            Ok(val) => Ok(BlockNumber(val)),
            Err(_) => Err(Felt252WrapperError::FromArrayError.into()),
        }
    }

    pub async fn get_last_block_hash(&self) -> Result<BlockHash> {
        let data = "0xe55d82f2";
        let block_hash = self.get_generic_call(data).await;
        match Felt252Wrapper::try_from(block_hash.unwrap()) {
            Ok(val) => Ok(BlockHash(val.into())),
            Err(_) => Err(Felt252WrapperError::FromArrayError.into()),
        }
    }
}

/// Suscribes for Ethereum state updates and applies new values
pub async fn sync(l1_url: Url) {
    let on_state_update = Arc::new(|state_update: EthereumStateUpdate| {
        log::info!("State updated: {:?}", state_update);
    });

    let client = EthereumClient::new(l1_url, Some(on_state_update)).unwrap();
    let current_state = Arc::new(Mutex::new(EthereumStateUpdate {
        state_root: client.get_last_state_root().await.unwrap(),
        block_number: client.get_last_block_number().await.unwrap(),
        block_hash: client.get_last_block_hash().await.unwrap(),
    }));

    log::info!("Started L1 verification on block: {:?}", client.get_last_block_number().await.unwrap());
    
    loop {
        let current_state = current_state.clone();
        let client = client.clone();

        tokio::spawn(async move {
            let mut state_guard = current_state.lock().await;
            let last_block_number = match client.get_last_block_number().await {
                Ok(number) => number,
                Err(e) => {
                    log::error!("Error fetching last block number: {}", e);
                    return;
                }
            };

            if last_block_number != state_guard.block_number {
                let state_root = client.get_last_state_root().await.unwrap_or_default();
                let block_hash = client.get_last_block_hash().await.unwrap_or_default();

                let new_state = EthereumStateUpdate {
                    state_root,
                    block_number: last_block_number,
                    block_hash,
                };

                log::info!("New state root detected on L1: {:?}", new_state.state_root);

                if let Some(callback) = &client.on_state_update {
                    callback(new_state.clone());
                }

                *state_guard = new_state;
            }
        }).await.unwrap();

        sleep(SLEEP_DURATION).await;
    }
}