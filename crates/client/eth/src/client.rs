use std::sync::Arc;

use alloy::{
    hex,
    primitives::{Address, B256},
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    rpc::types::Filter,
    sol,
    transports::http::{Client, Http},
};
use anyhow::{bail, Context};
use bitvec::macros::internal::funty::Fundamental;
use starknet_api::hash::StarkFelt;
use url::Url;

use crate::{
    client::StarknetCore::StarknetCoreInstance,
    config::L1StateUpdate,
    utils::{u256_to_starkfelt, LOG_STATE_UPDTATE_TOPIC},
};

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetCore,
    "src/abis/starknet_core.json"
);

pub struct EthereumClient {
    pub provider: Arc<ReqwestProvider>,
    pub l1_core_contract: StarknetCoreInstance<Http<Client>, RootProvider<Http<Client>>>,
}

impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url, l1_core_address: Address) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url);
        let core_contract = StarknetCore::new(l1_core_address, provider.clone());

        Ok(Self { provider: Arc::new(provider), l1_core_contract: core_contract })
    }

    /// Retrieves the latest Ethereum block number
    pub async fn get_latest_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.provider.get_block_number().await?.as_u64();
        Ok(block_number)
    }

    /// Get the block number of the last occurrence of a given event.
    pub async fn get_last_event_block_number(&self) -> anyhow::Result<u64> {
        let topic = B256::from_slice(&hex::decode(&LOG_STATE_UPDTATE_TOPIC[2..])?);
        let latest_block: u64 = self.get_latest_block_number().await?;

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(latest_block)
            .address(*self.l1_core_contract.address())
            .event_signature(topic);

        let logs = self.provider.get_logs(&filter).await?;

        if let Some(last_log) = logs.last() {
            let last_block: u64 = last_log.block_number.context("no block number in log")?;
            Ok(last_block)
        } else {
            bail!("no event found")
        }
    }

    /// Get the last Starknet block number verified on L1
    pub async fn get_last_verified_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.l1_core_contract.stateBlockNumber().call().await?;
        let last_block_number: u64 = (block_number._0).as_u64();
        Ok(last_block_number)
    }

    /// Get the last Starknet state root verified on L1
    pub async fn get_last_state_root(&self) -> anyhow::Result<StarkFelt> {
        let state_root = self.l1_core_contract.stateRoot().call().await?;
        u256_to_starkfelt(state_root._0)
    }

    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_verified_block_hash(&self) -> anyhow::Result<StarkFelt> {
        let block_hash = self.l1_core_contract.stateBlockHash().call().await?;
        u256_to_starkfelt(block_hash._0)
    }

    /// Get the last Starknet state update verified on the L1
    pub async fn get_initial_state(client: &EthereumClient) -> anyhow::Result<L1StateUpdate> {
        let block_number = client.get_last_verified_block_number().await?;
        let block_hash = client.get_last_verified_block_hash().await?;
        let global_root = client.get_last_state_root().await?;

        Ok(L1StateUpdate { global_root, block_number, block_hash })
    }
}

// #[cfg(test)]
// mod l1_sync_tests {
//     use ethers::contract::EthEvent;
//     use ethers::core::types::*;
//     use ethers::prelude::*;
//     use ethers::providers::Provider;
//     use tokio;
//     use url::Url;
//
//     use super::*;
//     use crate::l1::EthereumClient;
//
//     #[derive(Clone, Debug, EthEvent)]
//     pub struct Transfer {
//         #[ethevent(indexed)]
//         pub from: Address,
//         #[ethevent(indexed)]
//         pub to: Address,
//         pub tokens: U256,
//     }
//
//     pub mod eth_rpc {
//         pub const MAINNET: &str = "<ENTER-YOUR-RPC-URL-HERE>";
//     }
//
//     #[tokio::test]
//     #[ignore]
//     async fn test_starting_block() {
//         let url = Url::parse(eth_rpc::MAINNET).expect("Failed to parse URL");
//         let client = EthereumClient::new(url, H160::zero()).await.expect("Failed to create EthereumClient");
//
//         let start_block =
//             EthereumClient::get_last_event_block_number(&client).await.expect("Failed to get last event block number");
//         println!("The latest emission of the LogStateUpdate event was on block: {:?}", start_block);
//     }
//
//     #[tokio::test]
//     #[ignore]
//     async fn test_initial_state() {
//         let url = Url::parse(eth_rpc::MAINNET).expect("Failed to parse URL");
//         let client = EthereumClient::new(url, H160::zero()).await.expect("Failed to create EthereumClient");
//
//         let initial_state = EthereumClient::get_initial_state(&client).await.expect("Failed to get initial state");
//         assert!(!initial_state.global_root.bytes().is_empty(), "Global root should not be empty");
//         assert!(!initial_state.block_number > 0, "Block number should be greater than 0");
//         assert!(!initial_state.block_hash.bytes().is_empty(), "Block hash should not be empty");
//     }
//
//     #[tokio::test]
//     #[ignore]
//     async fn test_event_subscription() -> Result<(), Box<dyn std::error::Error>> {
//         abigen!(
//             IERC20,
//             r#"[
//                 function totalSupply() external view returns (uint256)
//                 function balanceOf(address account) external view returns (uint256)
//                 function transfer(address recipient, uint256 amount) external returns (bool)
//                 function allowance(address owner, address spender) external view returns (uint256)
//                 function approve(address spender, uint256 amount) external returns (bool)
//                 function transferFrom( address sender, address recipient, uint256 amount) external returns (bool)
//                 event Transfer(address indexed from, address indexed to, uint256 value)
//                 event Approval(address indexed owner, address indexed spender, uint256 value)
//             ]"#,
//         );
//
//         const WETH_ADDRESS: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
//
//         let provider = Provider::<Http>::try_from(eth_rpc::MAINNET)?;
//         let client = Arc::new(provider);
//         let address: Address = WETH_ADDRESS.parse()?;
//         let contract = IERC20::new(address, client);
//
//         let event = contract.event::<Transfer>().from_block(0).to_block(EthBlockNumber::Latest);
//
//         let mut event_stream = event.stream().await?;
//
//         while let Some(event_result) = event_stream.next().await {
//             match event_result {
//                 Ok(log) => {
//                     println!("Transfer event: {:?}", log);
//                 }
//                 Err(e) => println!("Error while listening for events: {:?}", e),
//             }
//         }
//
//         Ok(())
//     }
// }
