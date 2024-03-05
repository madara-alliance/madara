//! Contains the necessaries to perform an L1 verification of the state

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use ethers::contract::{abigen, EthEvent};
use ethers::providers::{Http, Middleware, Provider};
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::{Address, BlockNumber as EthBlockNumber, Filter, TransactionRequest, I256, U64};
use ethers::utils::hex::decode;
use futures::stream::StreamExt;
use lazy_static::lazy_static;
use mp_felt::Felt252Wrapper;
use primitive_types::{H256, U256};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use starknet_api::hash::StarkHash;

use crate::l2::STARKNET_STATE_UPDATE;
use crate::utility::{event_to_l1_state_update, get_state_update_at};
use crate::utils::constant::{starknet_core_address, LOG_STATE_UPDTATE_TOPIC};

lazy_static! {
    /// Shared latest L2 state update verified on L1
    pub static ref ETHEREUM_STATE_UPDATE: Arc<Mutex<L1StateUpdate>> = Arc::new(Mutex::new(L1StateUpdate {
        block_number: u64::default(),
        global_root: StarkHash::default(),
        block_hash: StarkHash::default(),
    }));
}

/// Contains the Starknet verified state on L1
#[derive(Debug, Clone, Deserialize)]
pub struct L1StateUpdate {
    pub block_number: u64,
    pub global_root: StarkHash,
    pub block_hash: StarkHash,
}

/// Starknet core LogStateUpdate event
#[derive(Clone, Debug, EthEvent, Deserialize)]
pub struct LogStateUpdate {
    #[ethevent(indexed)]
    pub global_root: U256,
    pub block_number: I256,
    pub block_hash: U256,
}

/// Ethereum client to interact with L1
#[derive(Clone)]
pub struct EthereumClient {
    provider: Arc<Provider<Http>>,
    url: Url,
}

/// Implementation of the Ethereum client to interact with L1
impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url) -> Result<Self> {
        let provider = Provider::<Http>::try_from(url.as_str())?;
        Ok(Self { provider: Arc::new(provider), url })
    }

    /// Get current RPC URL
    pub fn get_url(&self) -> String {
        self.url.as_str().to_string()
    }

    /// Call the Ethereum RPC endpoint with the given JSON-RPC payload
    pub async fn call_ethereum(&self, method: &str, params: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
        let response: Value = self.provider.request(method, params).await?;
        Ok(response)
    }

    /// Retrieves the latest Ethereum block number
    pub async fn get_latest_block_number(&self) -> Result<U64, Box<dyn std::error::Error>> {
        let block_number = self.provider.get_block_number().await?;
        Ok(block_number.as_u64().into())
    }

    /// Get the block number of the last occurrence of a given event.
    pub async fn get_last_event_block_number(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let topic = H256::from_slice(&hex::decode(&LOG_STATE_UPDTATE_TOPIC[2..])?);
        let address =
            Address::from_str(starknet_core_address::MAINNET).map_err(|e| format!("Failed to parse address: {}", e))?;
        let latest_block = self.get_latest_block_number().await.expect("Failed to retrieve latest block number");

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(EthBlockNumber::Latest)
            .address(vec![address])
            .topic0(topic);

        let logs = self.provider.get_logs(&filter).await?;

        if let Some(last_log) = logs.last() {
            let last_block = last_log.block_number.ok_or("No block number in log")?;
            Ok(last_block.as_u64())
        } else {
            Err("No events found".into())
        }
    }

    /// Get the last Starknet block number verified on L1
    pub async fn get_last_block_number(&self) -> Result<u64> {
        let data = decode("35befa5d")?;
        let to: Address = starknet_core_address::MAINNET.parse()?;
        let tx_request = TransactionRequest::new().to(to).data(data);
        let tx = TypedTransaction::Legacy(tx_request);
        let result = self.provider.call(&tx, None).await.expect("Failed to get last block number");
        let result_str = result.to_string();
        let hex_str = result_str.trim_start_matches("Bytes(0x").trim_end_matches(')').trim_start_matches("0x");

        let block_number = u64::from_str_radix(hex_str, 16).expect("Failed to parse block number");
        Ok(block_number)
    }

    /// Get the last Starknet state root verified on L1
    pub async fn get_last_state_root(&self) -> Result<StarkHash> {
        let data = decode("9588eca2")?;
        let to: Address = starknet_core_address::MAINNET.parse()?;
        let tx_request = TransactionRequest::new().to(to).data(data);
        let tx = TypedTransaction::Legacy(tx_request);
        let result = self.provider.call(&tx, None).await.expect("Failed to get last state root");
        Ok(StarkHash::from(Felt252Wrapper::from_hex_be(&result.to_string()).expect("Failed to parse state root")))
    }

    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_block_hash(&self) -> Result<StarkHash> {
        let data = decode("0x382d83e3")?;
        let to: Address = starknet_core_address::MAINNET.parse()?;
        let tx_request = TransactionRequest::new().to(to).data(data);
        let tx = TypedTransaction::Legacy(tx_request);
        let result = self.provider.call(&tx, None).await.expect("Failed to get last block hash");
        Ok(StarkHash::from(Felt252Wrapper::from_hex_be(&result.to_string()).expect("Failed to parse block hash")))
    }

    /// Get the last Starknet state update verified on the L1
    pub async fn get_initial_state(client: &EthereumClient) -> Result<L1StateUpdate, ()> {
        let block_number = client.get_last_block_number().await.map_err(|e| {
            log::error!("Failed to get last block number: {}", e);
        })?;
        let block_hash = client.get_last_block_hash().await.map_err(|e| {
            log::error!("Failed to get last block hash: {}", e);
        })?;
        let global_root = client.get_last_state_root().await.map_err(|e| {
            log::error!("Failed to get last state root: {}", e);
        })?;

        Ok(L1StateUpdate { global_root, block_number, block_hash })
    }

    /// Subscribes to the LogStateUpdate event from the Starknet core contract and store latest
    /// verified state
    pub async fn listen_and_update_state(&self, start_block: u64) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.provider.clone();
        let address: Address = starknet_core_address::MAINNET.parse().expect("Failed to parse Starknet core address");
        abigen!(
            StarknetCore,
            "crates/client/deoxys/src/utils/abis/starknet_core.json",
            event_derives(serde::Deserialize, serde::Serialize)
        );
        let contract = StarknetCore::new(address, client);

        let event_filter = contract.event::<LogStateUpdate>().from_block(start_block).to_block(EthBlockNumber::Latest);

        let mut event_stream = event_filter.stream().await.expect("Failed to initiate event stream");

        while let Some(event_result) = event_stream.next().await {
            match event_result {
                Ok(log) => {
                    println!("Log event in log format: {:?}", log.clone());
                    let format_event =
                        event_to_l1_state_update(log.clone()).expect("Failed to format event into an L1StateUpdate");
                    println!("LogStateUpdate event: {:?}, format event: {:?}", log, format_event.clone());
                    update_l1(format_event);
                }
                Err(e) => println!("Error while listening for events: {:?}", e),
            }
        }

        Ok(())
    }
}

/// Update the L1 state with the latest data
pub fn update_l1(state_update: L1StateUpdate) {
    log::info!(
        "🔄 Updated L1 head: Number: #{}, Hash: {}, Root: {}",
        state_update.block_number,
        state_update.block_hash,
        state_update.global_root
    );

    {
        let last_state_update = ETHEREUM_STATE_UPDATE.clone();
        let mut new_state_update = last_state_update.lock().unwrap();
        *new_state_update = state_update.clone();
    }
}

/// Verify the L1 state with the latest data
pub async fn verify_l1(state_update: L1StateUpdate, rpc_port: u16) -> Result<(), String> {
    let starknet_state_block_number = STARKNET_STATE_UPDATE.lock().map_err(|e| e.to_string())?.block_number;

    // Check if the node reached the latest verified state on Ethereum
    if state_update.block_number > starknet_state_block_number {
        return Err("🚨 L1 state verification failed: Node still syncing".into());
    }

    if state_update.block_number <= starknet_state_block_number {
        let current_state_update = get_state_update_at(rpc_port, state_update.block_number)
            .await
            .map_err(|e| format!("Error retrieving state update: {}", e))?;

        // Verifying Block Number, Block Hash and State Root against L2
        if current_state_update.block_number != state_update.block_number
            || current_state_update.global_root != state_update.global_root
            || current_state_update.block_hash != state_update.block_hash
        {
            return Err("🚨 L1 state verification failed: Verification mismatch".into());
        }

        log::info!(
            "✅ Verified L2 state via L1: #{}, Hash: {}, Root: {}",
            state_update.block_number,
            state_update.block_hash,
            state_update.global_root
        );
    }

    Ok(())
}

/// Syncronize with the L1 latest state updates
pub async fn sync(l1_url: Url) {
    let client = EthereumClient::new(l1_url).await.expect("Failed to create EthereumClient");

    log::info!("🚀 Subscribed to L1 state verification");

    // Get and store the latest verified state
    let initial_state = match EthereumClient::get_initial_state(&client).await {
        Ok(state) => state,
        Err(_) => return,
    };
    update_l1(initial_state);

    // Listen to LogStateUpdate (0x77552641) update and send changes continusly
    let start_block =
        EthereumClient::get_last_event_block_number(&client).await.expect("Failed to retrieve last event block number");
    EthereumClient::listen_and_update_state(&client, start_block).await.unwrap();
}

#[cfg(test)]
mod l1_sync_tests {
    use ethers::contract::EthEvent;
    use ethers::core::types::*;
    use ethers::prelude::*;
    use ethers::providers::Provider;
    use tokio;
    use url::Url;

    use super::*;
    use crate::l1::EthereumClient;

    #[derive(Clone, Debug, EthEvent)]
    pub struct Transfer {
        #[ethevent(indexed)]
        pub from: Address,
        #[ethevent(indexed)]
        pub to: Address,
        pub tokens: U256,
    }

    pub mod eth_rpc {
        pub const MAINNET: &str = "<ENTER-YOUR-RPC-URL-HERE>";
    }

    #[tokio::test]
    async fn test_starting_block() {
        let url = Url::parse(eth_rpc::MAINNET).expect("Failed to parse URL");
        let client = EthereumClient::new(url).await.expect("Failed to create EthereumClient");

        let start_block =
            EthereumClient::get_last_event_block_number(&client).await.expect("Failed to get last event block number");
        println!("The latest emission of the LogStateUpdate event was on block: {:?}", start_block);
    }

    #[tokio::test]
    async fn test_initial_state() {
        let url = Url::parse(eth_rpc::MAINNET).expect("Failed to parse URL");
        let client = EthereumClient::new(url).await.expect("Failed to create EthereumClient");

        let initial_state = EthereumClient::get_initial_state(&client).await.expect("Failed to get initial state");
        assert!(!initial_state.global_root.0.is_empty(), "Global root should not be empty");
        assert!(!initial_state.block_number > 0, "Block number should be greater than 0");
        assert!(!initial_state.block_hash.0.is_empty(), "Block hash should not be empty");
    }

    #[tokio::test]
    async fn test_event_subscription() -> Result<(), Box<dyn std::error::Error>> {
        abigen!(
            IERC20,
            r#"[
                function totalSupply() external view returns (uint256)
                function balanceOf(address account) external view returns (uint256)
                function transfer(address recipient, uint256 amount) external returns (bool)
                function allowance(address owner, address spender) external view returns (uint256)
                function approve(address spender, uint256 amount) external returns (bool)
                function transferFrom( address sender, address recipient, uint256 amount) external returns (bool)
                event Transfer(address indexed from, address indexed to, uint256 value)
                event Approval(address indexed owner, address indexed spender, uint256 value)
            ]"#,
        );

        const WETH_ADDRESS: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";

        let provider = Provider::<Http>::try_from(eth_rpc::MAINNET)?;
        let client = Arc::new(provider);
        let address: Address = WETH_ADDRESS.parse()?;
        let contract = IERC20::new(address, client);

        let event = contract.event::<Transfer>().from_block(0).to_block(EthBlockNumber::Latest);

        let mut event_stream = event.stream().await?;

        while let Some(event_result) = event_stream.next().await {
            match event_result {
                Ok(log) => {
                    println!("Transfer event: {:?}", log);
                }
                Err(e) => println!("Error while listening for events: {:?}", e),
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn listen_and_update_state() -> Result<(), Box<dyn std::error::Error>> {
        let client = EthereumClient::new(Url::parse(eth_rpc::MAINNET).expect("Failed to parse rpc url"))
            .await
            .expect("Failed to create EthereumClient");
        let start_block = EthereumClient::get_last_event_block_number(&client)
            .await
            .expect("Failed to retrieve last event block number");
        EthereumClient::listen_and_update_state(&client, start_block).await.unwrap();

        Ok(())
    }
}
