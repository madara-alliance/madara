//! Contains the necessaries to perform an L1 verification of the state

use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio::time::sleep;

use reqwest::Url;
use serde_json::Value;
use anyhow::Result;

type StateCommitment = String;
type BlockNumber = String;
type BlockHash = String;

const HTTP_OK: u16 = 200;

pub struct EthereumStateUpdate {
    pub state_root: StateCommitment,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

#[derive(Clone)]
pub struct EthereumClient {
    http: reqwest::Client,
    url: Url,
}

impl EthereumClient {
    pub fn new(url: Url) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::new(),
            url,
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

    pub async fn eth_call(&self, to: &str, data: &str) -> Result<String, anyhow::Error> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [
                {
                    "to": to,
                    "data": data
                },
                "latest"
            ],
            "id": 2
        });
    
        let response = self.call_ethereum(payload).await?;
        let result = response
            .as_str()
            .ok_or(anyhow::anyhow!("Invalid response"))?
            .to_string();
        Ok(result)
    }

    pub async fn get_state_root(&self) -> Result<String, anyhow::Error> {
        let to_address = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
        let data = "0x9588eca2";
        self.eth_call(to_address, data).await
    }

    pub async fn get_last_block_number(&self) -> Result<BlockNumber, anyhow::Error> {
        let to_address = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
        let data = "0x666bd0be";
        self.eth_call(to_address, data).await
    }

    pub async fn get_last_block_hash(&self) -> Result<BlockHash, anyhow::Error> {
        let to_address = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
        let data = "0xe55d82f2";
        self.eth_call(to_address, data).await
    }
}

pub async fn sync(l1_client_url: Url) {
    let url = l1_client_url;
    let client = EthereumClient::new(url)?;
    let current_state = Arc::new(Mutex::new(EthereumStateUpdate {
        state_root: String::new(),
        block_number: BlockNumber::default(),
        block_hash: BlockHash::default(),
    }));

    loop {
        let current_state = current_state.clone();
        let client = client.clone();

        tokio::spawn(async move {
            let mut state_guard = current_state.lock().await;
            let last_block_number = client.get_last_block_number().await.unwrap_or_default(); 

            if last_block_number != state_guard.block_number {
                let state_root = client.get_state_root().await.unwrap_or_default(); 
                let block_hash = client.get_last_block_hash().await.unwrap_or_default(); 

                state_guard.block_number = last_block_number;
                state_guard.state_root = state_root;
                state_guard.block_hash = block_hash;
            }
        }).await.unwrap_or_default(); 

        sleep(Duration::from_secs(1800)).await;
    }
}