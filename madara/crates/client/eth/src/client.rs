use crate::client::StarknetCoreContract::StarknetCoreContractInstance;
use crate::utils::u256_to_felt;
use alloy::sol_types::SolEvent;
use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    rpc::types::Filter,
    sol,
    transports::http::{Client, Http},
};
use mc_analytics::register_gauge_metric_instrument;
use opentelemetry::{global, KeyValue};
use opentelemetry::{global::Error, metrics::Gauge};

use anyhow::{bail, Context};
use bitvec::macros::internal::funty::Fundamental;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use url::Url;

#[derive(Clone, Debug)]
pub struct L1BlockMetrics {
    // L1 network metrics
    pub l1_block_number: Gauge<u64>,
    // gas price is also define in sync/metrics/block_metrics.rs but this would be the price from l1
    pub l1_gas_price_wei: Gauge<u64>,
    pub l1_gas_price_strk: Gauge<f64>,
}

impl L1BlockMetrics {
    pub fn register() -> Result<Self, Error> {
        let common_scope_attributes = vec![KeyValue::new("crate", "L1 Block")];
        let eth_meter = global::meter_with_version(
            "crates.l1block.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let l1_block_number = register_gauge_metric_instrument(
            &eth_meter,
            "l1_block_number".to_string(),
            "Gauge for madara L1 block number".to_string(),
            "".to_string(),
        );

        let l1_gas_price_wei = register_gauge_metric_instrument(
            &eth_meter,
            "l1_gas_price_wei".to_string(),
            "Gauge for madara L1 gas price in wei".to_string(),
            "".to_string(),
        );

        let l1_gas_price_strk = register_gauge_metric_instrument(
            &eth_meter,
            "l1_gas_price_strk".to_string(),
            "Gauge for madara L1 gas price in strk".to_string(),
            "".to_string(),
        );

        Ok(Self { l1_block_number, l1_gas_price_wei, l1_gas_price_strk })
    }
}

// abi taken from: https://etherscan.io/address/0x6e0acfdc3cf17a7f99ed34be56c3dfb93f464e24#code
// The official starknet core contract ^
sol!(
    #[sol(rpc)]
    #[derive(Debug)]
    StarknetCoreContract,
    "src/abis/starknet_core.json"
);

pub struct EthereumClient {
    pub provider: Arc<ReqwestProvider>,
    pub l1_core_contract: StarknetCoreContractInstance<Http<Client>, RootProvider<Http<Client>>>,
    pub l1_block_metrics: L1BlockMetrics,
}

impl Clone for EthereumClient {
    fn clone(&self) -> Self {
        EthereumClient {
            provider: Arc::clone(&self.provider),
            l1_core_contract: self.l1_core_contract.clone(),
            l1_block_metrics: self.l1_block_metrics.clone(),
        }
    }
}

impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url, l1_core_address: Address, l1_block_metrics: L1BlockMetrics) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url);

        EthereumClient::assert_core_contract_exists(&provider, l1_core_address).await?;

        let core_contract = StarknetCoreContract::new(l1_core_address, provider.clone());

        Ok(Self { provider: Arc::new(provider), l1_core_contract: core_contract, l1_block_metrics })
    }

    /// Assert that L1 Core contract exists by checking its bytecode.
    async fn assert_core_contract_exists(
        provider: &RootProvider<Http<Client>>,
        l1_core_address: Address,
    ) -> anyhow::Result<()> {
        let l1_core_contract_bytecode = provider.get_code_at(l1_core_address).await?;
        if l1_core_contract_bytecode.is_empty() {
            bail!("The L1 Core Contract could not be found. Check that the L2 chain matches the L1 RPC endpoint.");
        }
        Ok(())
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
    pub async fn get_last_state_root(&self) -> anyhow::Result<Felt> {
        let state_root = self.l1_core_contract.stateRoot().call().await?;
        u256_to_felt(state_root._0)
    }

    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_verified_block_hash(&self) -> anyhow::Result<Felt> {
        let block_hash = self.l1_core_contract.stateBlockHash().call().await?;
        u256_to_felt(block_hash._0)
    }
}

#[cfg(test)]
pub mod eth_client_getter_test {
    use super::*;
    use alloy::{
        node_bindings::{Anvil, AnvilInstance},
        primitives::U256,
    };

    use std::ops::{Deref, Range};
    use std::sync::Mutex;
    use tokio;

    // https://etherscan.io/tx/0xcadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81#eventlog
    // The txn we are referring to it is here ^
    const L1_BLOCK_NUMBER: u64 = 20395662;
    const CORE_CONTRACT_ADDRESS: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    const L2_BLOCK_NUMBER: u64 = 662703;
    const L2_BLOCK_HASH: &str = "563216050958639290223177746678863910249919294431961492885921903486585884664";
    const L2_STATE_ROOT: &str = "1456190284387746219409791261254265303744585499659352223397867295223408682130";

    lazy_static::lazy_static! {
        static ref FORK_URL: String = std::env::var("ETH_FORK_URL").expect("ETH_FORK_URL not set");
    }

    const PORT_RANGE: Range<u16> = 19500..20000;

    struct AvailablePorts<I: Iterator<Item = u16>> {
        to_reuse: Vec<u16>,
        next: I,
    }

    lazy_static::lazy_static! {
        static ref AVAILABLE_PORTS: Mutex<AvailablePorts<Range<u16>>> = Mutex::new(AvailablePorts { to_reuse: vec![], next: PORT_RANGE });
    }
    pub struct AnvilPortNum(pub u16);
    impl Drop for AnvilPortNum {
        fn drop(&mut self) {
            let mut guard = AVAILABLE_PORTS.lock().expect("poisoned lock");
            guard.to_reuse.push(self.0);
        }
    }

    pub fn get_port() -> AnvilPortNum {
        let mut guard = AVAILABLE_PORTS.lock().expect("poisoned lock");
        if let Some(el) = guard.to_reuse.pop() {
            return AnvilPortNum(el);
        }
        AnvilPortNum(guard.next.next().expect("no more port to use"))
    }

    static ANVIL: Mutex<Option<Arc<AnvilInstance>>> = Mutex::new(None);

    /// Wrapper for an Anvil instance that automatically cleans up when all handles are dropped
    pub struct AnvilHandle {
        instance: Arc<AnvilInstance>,
    }

    impl Drop for AnvilHandle {
        fn drop(&mut self) {
            let mut guard = ANVIL.lock().expect("poisoned lock");
            // Check if this Arc is the last one (strong_count == 2 because of our reference
            // and the one in the static)
            if Arc::strong_count(&self.instance) == 2 {
                println!("Cleaning up Anvil instance");
                *guard = None;
            }
        }
    }

    impl Deref for AnvilHandle {
        type Target = AnvilInstance;

        fn deref(&self) -> &Self::Target {
            &self.instance
        }
    }

    pub fn get_shared_anvil() -> AnvilHandle {
        let mut guard = ANVIL.lock().expect("poisoned lock");
        if guard.is_none() {
            *guard = Some(Arc::new(create_anvil_instance()));
        }
        AnvilHandle { instance: Arc::clone(guard.as_ref().unwrap()) }
    }

    pub fn create_anvil_instance() -> AnvilInstance {
        let port = get_port();
        let anvil = Anvil::new()
            .fork(FORK_URL.clone())
            .fork_block_number(L1_BLOCK_NUMBER)
            .port(port.0)
            .timeout(60_000)
            .try_spawn()
            .expect("failed to spawn anvil instance");
        println!("Anvil started and running at `{}`", anvil.endpoint());
        anvil
    }

    pub fn create_ethereum_client(url: Option<&str>) -> EthereumClient {
        let rpc_url: Url = url.unwrap_or("http://localhost:8545").parse().expect("issue while parsing URL");

        let provider = ProviderBuilder::new().on_http(rpc_url.clone());
        let address = Address::parse_checksummed(CORE_CONTRACT_ADDRESS, None).unwrap();
        let contract = StarknetCoreContract::new(address, provider.clone());

        let l1_block_metrics = L1BlockMetrics::register().unwrap();

        EthereumClient { provider: Arc::new(provider), l1_core_contract: contract.clone(), l1_block_metrics }
    }

    #[tokio::test]
    async fn fail_create_new_client_invalid_core_contract() {
        let anvil = get_shared_anvil();
        // Sepolia core contract instead of mainnet
        const INVALID_CORE_CONTRACT_ADDRESS: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";

        let rpc_url: Url = anvil.endpoint_url();

        let core_contract_address = Address::parse_checksummed(INVALID_CORE_CONTRACT_ADDRESS, None).unwrap();
        let l1_block_metrics = L1BlockMetrics::register().unwrap();

        let new_client_result = EthereumClient::new(rpc_url, core_contract_address, l1_block_metrics).await;
        assert!(new_client_result.is_err(), "EthereumClient::new should fail with an invalid core contract address");
    }

    #[tokio::test]
    async fn get_latest_block_number_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_number =
            eth_client.provider.get_block_number().await.expect("issue while fetching the block number").as_u64();
        assert_eq!(block_number, L1_BLOCK_NUMBER, "provider unable to get the correct block number");
    }

    #[tokio::test]
    async fn get_last_event_block_number_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_number = eth_client
            .get_last_event_block_number::<StarknetCoreContract::LogStateUpdate>()
            .await
            .expect("issue while getting the last block number with given event");
        assert_eq!(block_number, L1_BLOCK_NUMBER, "block number with given event not matching");
    }

    #[tokio::test]
    async fn get_last_verified_block_hash_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_hash =
            eth_client.get_last_verified_block_hash().await.expect("issue while getting the last verified block hash");
        let expected = u256_to_felt(U256::from_str_radix(L2_BLOCK_HASH, 10).unwrap()).unwrap();
        assert_eq!(block_hash, expected, "latest block hash not matching");
    }

    #[tokio::test]
    async fn get_last_state_root_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let state_root = eth_client.get_last_state_root().await.expect("issue while getting the state root");
        let expected = u256_to_felt(U256::from_str_radix(L2_STATE_ROOT, 10).unwrap()).unwrap();
        assert_eq!(state_root, expected, "latest block state root not matching");
    }

    #[tokio::test]
    async fn get_last_verified_block_number_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_number = eth_client.get_last_verified_block_number().await.expect("issue");
        assert_eq!(block_number, L2_BLOCK_NUMBER, "verified block number not matching");
    }
}
