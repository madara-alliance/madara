use crate::client::{ClientTrait, ClientType, CoreContractInstance};
use crate::eth::StarknetCoreContract::StarknetCoreContractInstance;
use crate::gas_price::L1BlockMetrics;
use crate::state_update::{update_l1, StateUpdate};
use crate::utils::{convert_log_state_update, u256_to_felt};
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{Address, FixedBytes};
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::transports::http::{Client, Http};
use anyhow::{bail, Context};
use async_trait::async_trait;
use bitvec::macros::internal::funty::Fundamental;
use futures::StreamExt;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use url::Url;

// abi taken from: https://etherscan.io/address/0x6e0acfdc3cf17a7f99ed34be56c3dfb93f464e24#code
// The official starknet core contract ^
sol!(
    #[sol(rpc)]
    #[derive(Debug)]
    StarknetCoreContract,
    "src/eth/starknet_core.json"
);

const ERR_ARCHIVE: &str =
    "Failed to watch event filter - Ensure you are using an L1 RPC endpoint that points to an archive node";

pub struct EthereumClient {
    pub provider: Arc<ReqwestProvider>,
    pub l1_core_contract: StarknetCoreContractInstance<Http<Client>, RootProvider<Http<Client>>>,
    pub l1_block_metrics: L1BlockMetrics,
}

#[derive(Clone)]
pub struct EthereumClientConfig {
    pub url: Url,
    pub l1_core_address: Address,
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

#[async_trait]
impl ClientTrait for EthereumClient {
    type Provider = RootProvider<Http<Client>>;
    type Config = EthereumClientConfig;

    fn get_l1_block_metrics(&self) -> &L1BlockMetrics {
        &self.l1_block_metrics
    }

    fn get_core_contract_instance(&self) -> CoreContractInstance {
        CoreContractInstance::Ethereum(self.l1_core_contract.clone())
    }

    fn get_client_type(&self) -> ClientType {
        ClientType::ETH
    }

    /// Create a new EthereumClient instance with the given RPC URL
    async fn new(config: EthereumClientConfig) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(config.url);
        // Checking if core contract exists on l1
        let l1_core_contract_bytecode = provider.get_code_at(config.l1_core_address).await?;
        if l1_core_contract_bytecode.is_empty() {
            bail!("The L1 Core Contract could not be found. Check that the L2 chain matches the L1 RPC endpoint.");
        }
        let core_contract = StarknetCoreContract::new(config.l1_core_address, provider.clone());
        Ok(Self {
            provider: Arc::new(provider),
            l1_core_contract: core_contract,
            l1_block_metrics: config.l1_block_metrics,
        })
    }

    /// Retrieves the latest Ethereum block number
    async fn get_latest_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.provider.get_block_number().await?.as_u64();
        Ok(block_number)
    }

    /// Get the block number of the last occurrence of a given event.
    async fn get_last_event_block_number(&self) -> anyhow::Result<u64> {
        let latest_block: u64 = self.get_latest_block_number().await?;

        // Assuming an avg Block time of 15sec we check for a LogStateUpdate occurence in the last ~24h
        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(latest_block)
            .address(*self.l1_core_contract.address());

        let logs = self.provider.get_logs(&filter).await?;

        let filtered_logs = logs
            .into_iter()
            .filter_map(|log| log.log_decode::<StarknetCoreContract::LogStateUpdate>().ok())
            .collect::<Vec<_>>();

        if let Some(last_log) = filtered_logs.last() {
            let last_block: u64 = last_log.block_number.context("no block number in log")?;
            Ok(last_block)
        } else {
            bail!("no event found")
        }
    }

    /// Get the last Starknet block number verified on L1
    async fn get_last_verified_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.l1_core_contract.stateBlockNumber().call().await?;
        let last_block_number: u64 = (block_number._0).as_u64();
        Ok(last_block_number)
    }

    /// Get the last Starknet state root verified on L1
    async fn get_last_state_root(&self) -> anyhow::Result<Felt> {
        let state_root = self.l1_core_contract.stateRoot().call().await?;
        u256_to_felt(state_root._0)
    }

    /// Get the last Starknet block hash verified on L1
    async fn get_last_verified_block_hash(&self) -> anyhow::Result<Felt> {
        let block_hash = self.l1_core_contract.stateBlockHash().call().await?;
        u256_to_felt(block_hash._0)
    }

    async fn get_initial_state(&self) -> anyhow::Result<StateUpdate> {
        let block_number = self.get_last_verified_block_number().await?;
        let block_hash = self.get_last_verified_block_hash().await?;
        let global_root = self.get_last_state_root().await?;

        Ok(StateUpdate { global_root, block_number, block_hash })
    }

    async fn listen_for_update_state_events(
        &self,
        backend: Arc<MadaraBackend>,
        mut ctx: ServiceContext,
    ) -> anyhow::Result<()> {
        // Listen to LogStateUpdate (0x77552641) update and send changes continuously
        let contract_instance = self.get_core_contract_instance();
        let event_filter = contract_instance.event_filter::<StarknetCoreContract::LogStateUpdate>();

        let mut event_stream = match ctx.run_until_cancelled(event_filter?.watch()).await {
            Some(res) => res.context(ERR_ARCHIVE)?.into_stream(),
            None => return anyhow::Ok(()),
        };

        while let Some(Some(event_result)) = ctx.run_until_cancelled(event_stream.next()).await {
            let log = event_result.context("listening for events")?;
            let format_event: StateUpdate =
                convert_log_state_update(log.0.clone()).context("formatting event into an L1StateUpdate")?;
            update_l1(&backend, format_event, self.get_l1_block_metrics())?;
        }

        Ok(())
    }

    async fn get_eth_gas_prices(&self) -> anyhow::Result<(u128, u128)> {
        let block_number = self.get_latest_block_number().await?;
        let fee_history = self.provider.get_fee_history(300, BlockNumberOrTag::Number(block_number), &[]).await?;

        // The RPC responds with 301 elements for some reason. It's also just safer to manually
        // take the last 300. We choose 300 to get average gas caprice for last one hour (300 * 12 sec block
        // time).
        let (_, blob_fee_history_one_hour) =
            fee_history.base_fee_per_blob_gas.split_at(fee_history.base_fee_per_blob_gas.len().max(300) - 300);

        let avg_blob_base_fee = if !blob_fee_history_one_hour.is_empty() {
            blob_fee_history_one_hour.iter().sum::<u128>() / blob_fee_history_one_hour.len() as u128
        } else {
            0 // in case blob_fee_history_one_hour has 0 length
        };

        let eth_gas_price = fee_history.base_fee_per_gas.last().context("Getting eth gas price")?;
        Ok((*eth_gas_price, avg_blob_base_fee))
    }

    /// Get cancellation status of an L1 to L2 message
    ///
    /// This function query the core contract to know if a L1->L2 message has been cancelled
    /// # Arguments
    ///
    /// - msg_hash : Hash of L1 to L2 message
    ///
    /// # Return
    ///
    /// - A felt representing a timestamp :
    ///     - 0 if the message has not been cancelled
    ///     - timestamp of the cancellation if it has been cancelled
    /// - An Error if the call fail
    async fn get_l1_to_l2_message_cancellations(&self, msg_hash: FixedBytes<32>) -> anyhow::Result<Felt> {
        //l1ToL2MessageCancellations
        let cancellation_timestamp = self.l1_core_contract.l1ToL2MessageCancellations(msg_hash).call().await?;
        u256_to_felt(cancellation_timestamp._0)
    }
}

#[cfg(test)]
pub mod eth_client_getter_test {
    use super::*;
    use alloy::{
        node_bindings::{Anvil, AnvilInstance},
        primitives::U256,
    };

    use crate::gas_price::L1BlockMetrics;
    use serial_test::serial;
    use std::ops::Range;
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

    pub fn get_shared_anvil() -> Arc<AnvilInstance> {
        let mut anvil = ANVIL.lock().expect("poisoned lock");
        if anvil.is_none() {
            *anvil = Some(Arc::new(create_anvil_instance()));
        }
        Arc::clone(anvil.as_ref().unwrap())
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

    #[serial]
    #[tokio::test]
    async fn fail_create_new_client_invalid_core_contract() {
        let anvil = get_shared_anvil();
        // Sepolia core contract instead of mainnet
        const INVALID_CORE_CONTRACT_ADDRESS: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";

        let rpc_url: Url = anvil.endpoint_url();

        let core_contract_address = Address::parse_checksummed(INVALID_CORE_CONTRACT_ADDRESS, None).unwrap();
        let l1_block_metrics = L1BlockMetrics::register().unwrap();

        let ethereum_client_config =
            EthereumClientConfig { url: rpc_url, l1_core_address: core_contract_address, l1_block_metrics };

        let new_client_result = EthereumClient::new(ethereum_client_config).await;
        assert!(new_client_result.is_err(), "EthereumClient::new should fail with an invalid core contract address");
    }

    #[serial]
    #[tokio::test]
    async fn get_latest_block_number_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_number =
            eth_client.provider.get_block_number().await.expect("issue while fetching the block number").as_u64();
        assert_eq!(block_number, L1_BLOCK_NUMBER, "provider unable to get the correct block number");
    }

    #[serial]
    #[tokio::test]
    async fn get_last_event_block_number_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_number = eth_client
            .get_last_event_block_number()
            .await
            .expect("issue while getting the last block number with given event");
        assert_eq!(block_number, L1_BLOCK_NUMBER, "block number with given event not matching");
    }

    #[serial]
    #[tokio::test]
    async fn get_last_verified_block_hash_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_hash =
            eth_client.get_last_verified_block_hash().await.expect("issue while getting the last verified block hash");
        let expected = u256_to_felt(U256::from_str_radix(L2_BLOCK_HASH, 10).unwrap()).unwrap();
        assert_eq!(block_hash, expected, "latest block hash not matching");
    }

    #[serial]
    #[tokio::test]
    async fn get_last_state_root_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let state_root = eth_client.get_last_state_root().await.expect("issue while getting the state root");
        let expected = u256_to_felt(U256::from_str_radix(L2_STATE_ROOT, 10).unwrap()).unwrap();
        assert_eq!(state_root, expected, "latest block state root not matching");
    }

    #[serial]
    #[tokio::test]
    async fn get_last_verified_block_number_works() {
        let anvil = get_shared_anvil();
        let eth_client = create_ethereum_client(Some(anvil.endpoint().as_str()));
        let block_number = eth_client.get_last_verified_block_number().await.expect("issue");
        assert_eq!(block_number, L2_BLOCK_NUMBER, "verified block number not matching");
    }
}
