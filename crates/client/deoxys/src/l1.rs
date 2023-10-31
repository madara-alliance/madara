//! Contains the necessaries to perform an L1 verification of the state

use reqwest::Url;
use serde_json::Value;
use anyhow::Result;

const HTTP_OK: u16 = 200;

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

    pub async fn get_latest_block_number(&self) -> Result<u64> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });

        let response = self.call_ethereum(payload).await?;
        let block_number_hex = response.as_str().ok_or(anyhow::anyhow!("Invalid response"))?;
        let block_number = u64::from_str_radix(&block_number_hex[2..], 16)?;
        Ok(block_number)
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
}

pub async fn verify() -> Result<()> {
    let url = Url::parse("https://eth-mainnet.g.alchemy.com/v2/CDPivBjTjgi1b1Qov1IrL6bBPxGwnAgg")?;
    let client = EthereumClient::new(url)?;

    let block_number = client.get_latest_block_number().await?;
    println!("Latest block number: {}", block_number);

    let result = client.eth_call("0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4", "0x9588eca2").await?;
    println!("eth_call result: {}", result);

    Ok(())
}