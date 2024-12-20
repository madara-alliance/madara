use crate::eth::StarknetCoreContract::StarknetCoreContractInstance;
use crate::gas_price::L1BlockMetrics;
use crate::state_update::StateUpdate;
use alloy::contract::Event;
use alloy::primitives::FixedBytes;
use alloy::providers::RootProvider;
use alloy::sol_types::SolEvent;
use alloy::transports::http::{Client, Http};
use async_trait::async_trait;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub enum ClientType {
    ETH,
    STARKNET,
}

pub enum CoreContractInstance {
    Ethereum(StarknetCoreContractInstance<Http<Client>, RootProvider<Http<Client>>>),
    Starknet(Felt),
}

impl CoreContractInstance {
    #[allow(clippy::type_complexity)]
    pub fn event_filter<T: SolEvent>(&self) -> anyhow::Result<Event<Http<Client>, &RootProvider<Http<Client>>, T>> {
        match self {
            CoreContractInstance::Ethereum(contract) => Ok(contract.event_filter()),
            CoreContractInstance::Starknet(_) => Err(anyhow::anyhow!("Starknet doesn't support event filters")),
        }
    }
}

#[async_trait]
pub trait ClientTrait: Send + Sync {
    // Provider type used by the implementation
    type Provider;
    // Configuration type used for initialization
    type Config;

    // Basic getter functions
    fn get_l1_block_metrics(&self) -> &L1BlockMetrics;
    fn get_core_contract_instance(&self) -> CoreContractInstance;
    fn get_client_type(&self) -> ClientType;

    // Create a new instance of the client
    async fn new(config: Self::Config) -> anyhow::Result<Self>
    where
        Self: Sized;

    // Get the latest block number
    async fn get_latest_block_number(&self) -> anyhow::Result<u64>;

    // Get the block number of the last occurrence of a specific event
    async fn get_last_event_block_number(&self) -> anyhow::Result<u64>;

    // Get the last verified block number
    async fn get_last_verified_block_number(&self) -> anyhow::Result<u64>;

    // Get the last state root
    // - change this to Felt in implementation
    // - write tests for conversion to Felt from <native-type>
    async fn get_last_state_root(&self) -> anyhow::Result<Felt>;

    // Get the last verified block hash
    async fn get_last_verified_block_hash(&self) -> anyhow::Result<Felt>;

    // Get initial state from client
    async fn get_initial_state(&self) -> anyhow::Result<StateUpdate>;
    async fn listen_for_update_state_events(
        &self,
        backend: Arc<MadaraBackend>,
        ctx: ServiceContext,
    ) -> anyhow::Result<()>;

    // get gas prices
    async fn get_eth_gas_prices(&self) -> anyhow::Result<(u128, u128)>;

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
    async fn get_l1_to_l2_message_cancellations(&self, msg_hash: FixedBytes<32>) -> anyhow::Result<Felt>;
}
