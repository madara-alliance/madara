use std::sync::Arc;

use alloy::sol_types::SolEvent;
use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    rpc::types::Filter,
    sol,
    transports::http::{Client, Http},
};
use anyhow::{bail, Context};
use bitvec::macros::internal::funty::Fundamental;
use starknet_api::hash::StarkFelt;
use url::Url;

use crate::client::StarknetCoreContract::StarknetCoreContractInstance;
use crate::{config::L1StateUpdate, utils::u256_to_starkfelt};

// abi taken from: https://etherscan.io/address/0x6e0acfdc3cf17a7f99ed34be56c3dfb93f464e24#code
// The official starknet core contract ^
sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetCoreContract,
    "src/abis/starknet_core.json"
);

pub struct EthereumClient {
    pub provider: Arc<ReqwestProvider>,
    pub l1_core_contract: StarknetCoreContractInstance<Http<Client>, RootProvider<Http<Client>>>,
}

impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url, l1_core_address: Address) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url);
        let core_contract = StarknetCoreContract::new(l1_core_address, provider.clone());

        Ok(Self { provider: Arc::new(provider), l1_core_contract: core_contract })
    }

    /// Retrieves the latest Ethereum block number
    pub async fn get_latest_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.provider.get_block_number().await?.as_u64();
        Ok(block_number)
    }

    /// Get the block number of the last occurrence of a given event.
    pub async fn get_last_event_block_number<T: SolEvent>(&self) -> anyhow::Result<u64> {
        let latest_block: u64 = self.get_latest_block_number().await?;

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(latest_block)
            .address(*self.l1_core_contract.address());

        let logs = self.provider.get_logs(&filter).await?;

        let filtered_logs = logs.into_iter().filter_map(|log| log.log_decode::<T>().ok()).collect::<Vec<_>>();

        if let Some(last_log) = filtered_logs.last() {
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

#[cfg(test)]
mod eth_client_getter_test {
    use super::*;
    use alloy::node_bindings::{Anvil, AnvilInstance};
    use alloy::primitives::{address, U256};
    use rstest::*;
    use tokio;

    #[fixture]
    #[once]
    fn anvil() -> AnvilInstance {
        let anvil = Anvil::new().fork("https://eth.merkle.io").fork_block_number(20395662).spawn();
        anvil
    }

    #[fixture]
    #[once]
    fn eth_client(anvil: &AnvilInstance) -> EthereumClient {
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider = ProviderBuilder::new().on_http(rpc_url.clone());
        let contract =
            StarknetCoreContract::new(address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4"), provider.clone());

        EthereumClient { provider: Arc::new(provider), l1_core_contract: contract.clone() }
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_latest_block_number(eth_client: &EthereumClient) {
        let block_number =
            eth_client.provider.get_block_number().await.expect("issue while fetching the block number").as_u64();
        assert_eq!(block_number, 20395662, "provider unable to get the correct block number");
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_last_event_block_number(eth_client: &EthereumClient) {
        let block_number = eth_client
            .get_last_event_block_number::<StarknetCoreContract::LogStateUpdate>()
            .await
            .expect("issue while getting the last block number with given event");
        assert_eq!(block_number, 20395662, "block number with given event not matching");
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_last_verified_block_hash(eth_client: &EthereumClient) {
        let block_hash =
            eth_client.get_last_verified_block_hash().await.expect("issue while getting the last verified block hash");
        let expected = u256_to_starkfelt(
            U256::from_str_radix("563216050958639290223177746678863910249919294431961492885921903486585884664", 10)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(block_hash, expected, "latest block hash not matching");
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_last_state_root(eth_client: &EthereumClient) {
        let state_root = eth_client.get_last_state_root().await.expect("issue while getting the state root");
        let expected = u256_to_starkfelt(
            U256::from_str_radix("1456190284387746219409791261254265303744585499659352223397867295223408682130", 10)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(state_root, expected, "latest block state root not matching");
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_last_verified_block_number(eth_client: &EthereumClient) {
        let block_number = eth_client.get_last_verified_block_number().await.expect("issue");
        assert_eq!(block_number, 662703, "verified block number not matching");
    }
}
