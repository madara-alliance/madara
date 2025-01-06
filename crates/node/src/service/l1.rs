use crate::cli::l1::{L1SyncParams, MadaraSettlementLayer};
use alloy::primitives::Address;
use anyhow::Context;
use futures::Stream;
use mc_db::{DatabaseService, MadaraBackend};
use mc_mempool::{GasPriceProvider, Mempool};
use mc_settlement_client::client::ClientTrait;
use mc_settlement_client::eth::event::EthereumEventStream;
use mc_settlement_client::eth::{EthereumClient, EthereumClientConfig};
use mc_settlement_client::gas_price::L1BlockMetrics;
use mc_settlement_client::messaging::sync::CommonMessagingEventData;
use mc_settlement_client::starknet::event::StarknetEventStream;
use mc_settlement_client::starknet::{StarknetClient, StarknetClientConfig};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use starknet_api::core::ChainId;
use starknet_core::types::Felt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

pub struct L1SyncService<C: 'static, S: 'static>
where
    C: Clone,
    S: Send + Stream<Item = Option<anyhow::Result<CommonMessagingEventData>>>,
{
    db_backend: Arc<MadaraBackend>,
    settlement_client: Option<Arc<Box<dyn ClientTrait<Config = C, StreamType = S>>>>,
    l1_gas_provider: GasPriceProvider,
    chain_id: ChainId,
    gas_price_sync_disabled: bool,
    gas_price_poll: Duration,
    mempool: Arc<Mempool>,
}

pub type EthereumSyncService = L1SyncService<EthereumClientConfig, EthereumEventStream>;
pub type StarknetSyncService = L1SyncService<StarknetClientConfig, StarknetEventStream>;

// Implementation for Ethereum
impl EthereumSyncService {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: &L1SyncParams,
        db: &DatabaseService,
        l1_gas_provider: GasPriceProvider,
        chain_id: ChainId,
        l1_core_address: String,
        authority: bool,
        devnet: bool,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<Self> {
        let settlement_client = if !config.l1_sync_disabled && (config.l1_endpoint.is_some() || !devnet) {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                let core_address = Address::from_str(l1_core_address.as_str())?;
                let l1_block_metrics = L1BlockMetrics::register().expect("Registering metrics");
                let client = EthereumClient::new(EthereumClientConfig {
                    url: l1_rpc_url.clone(),
                    l1_core_address: core_address,
                    l1_block_metrics,
                })
                .await
                .context("Creating ethereum client")?;

                let client_converted: Box<
                    dyn ClientTrait<Config = EthereumClientConfig, StreamType = EthereumEventStream>,
                > = Box::new(client);
                Some(Arc::new(client_converted))
            } else {
                anyhow::bail!(
                    "No Ethereum endpoint provided. Use --l1-endpoint <RPC URL> or disable with --no-l1-sync."
                );
            }
        } else {
            None
        };

        Self::create_service(config, db, settlement_client, l1_gas_provider, chain_id, authority, devnet, mempool).await
    }
}

// Implementation for Starknet
impl StarknetSyncService {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: &L1SyncParams,
        db: &DatabaseService,
        l1_gas_provider: GasPriceProvider,
        chain_id: ChainId,
        l1_core_address: String,
        authority: bool,
        devnet: bool,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<Self> {
        let settlement_client = if !config.l1_sync_disabled && (config.l1_endpoint.is_some() || !devnet) {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                let core_address = Felt::from_str(l1_core_address.as_str())?;
                let l1_block_metrics = L1BlockMetrics::register().expect("Registering metrics");
                let client = StarknetClient::new(StarknetClientConfig {
                    url: l1_rpc_url.clone(),
                    l2_contract_address: core_address,
                    l1_block_metrics,
                })
                .await
                .context("Creating starknet client")?;

                // StarknetClientConfig, Arc<JsonRpcClient<HttpTransport>>, Felt
                let client_converted: Box<
                    dyn ClientTrait<Config = StarknetClientConfig, StreamType = StarknetEventStream>,
                > = Box::new(client);
                Some(Arc::new(client_converted))
            } else {
                anyhow::bail!(
                    "No Starknet endpoint provided. Use --l1-endpoint <RPC URL> or disable with --no-l1-sync."
                );
            }
        } else {
            None
        };

        Self::create_service(config, db, settlement_client, l1_gas_provider, chain_id, authority, devnet, mempool).await
    }
}

// Shared implementation for both services
impl<C: Clone, S> L1SyncService<C, S>
where
    C: Clone + 'static,
    S: Send + Stream<Item = Option<anyhow::Result<CommonMessagingEventData>>> + 'static,
{
    #[allow(clippy::too_many_arguments)]
    async fn create_service(
        config: &L1SyncParams,
        db: &DatabaseService,
        settlement_client: Option<Arc<Box<dyn ClientTrait<Config = C, StreamType = S>>>>,
        l1_gas_provider: GasPriceProvider,
        chain_id: ChainId,
        authority: bool,
        devnet: bool,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<Self> {
        let gas_price_sync_enabled =
            authority && !devnet && (config.gas_price.is_none() || config.blob_gas_price.is_none());
        let gas_price_poll = config.gas_price_poll;

        if gas_price_sync_enabled {
            let settlement_client =
                settlement_client.clone().context("L1 gas prices require the service to be enabled...")?;
            tracing::info!("⏳ Getting initial L1 gas prices");
            mc_settlement_client::gas_price::gas_price_worker_once(settlement_client, &l1_gas_provider, gas_price_poll)
                .await
                .context("Getting initial gas prices")?;
        }

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            settlement_client,
            l1_gas_provider,
            chain_id,
            gas_price_sync_disabled: !gas_price_sync_enabled,
            gas_price_poll,
            mempool,
        })
    }

    // Factory method to create the appropriate service
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        config: &L1SyncParams,
        db: &DatabaseService,
        l1_gas_provider: GasPriceProvider,
        chain_id: ChainId,
        l1_core_address: String,
        authority: bool,
        devnet: bool,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<Box<dyn Service>> {
        match config.settlement_layer {
            MadaraSettlementLayer::Eth => Ok(Box::new(
                EthereumSyncService::new(
                    config,
                    db,
                    l1_gas_provider,
                    chain_id,
                    l1_core_address,
                    authority,
                    devnet,
                    mempool,
                )
                .await?,
            )),
            MadaraSettlementLayer::Starknet => Ok(Box::new(
                StarknetSyncService::new(
                    config,
                    db,
                    l1_gas_provider,
                    chain_id,
                    l1_core_address,
                    authority,
                    devnet,
                    mempool,
                )
                .await?,
            )),
        }
    }
}

#[async_trait::async_trait]
impl<C, S> Service for L1SyncService<C, S>
where
    C: Clone,
    S: Send + Stream<Item = Option<anyhow::Result<CommonMessagingEventData>>>,
{
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if let Some(settlement_client) = &self.settlement_client {
            let db_backend = Arc::clone(&self.db_backend);
            let settlement_client = Arc::clone(settlement_client);
            let chain_id = self.chain_id.clone();
            let l1_gas_provider = self.l1_gas_provider.clone();
            let gas_price_sync_disabled = self.gas_price_sync_disabled;
            let gas_price_poll = self.gas_price_poll;
            let mempool = Arc::clone(&self.mempool);

            runner.service_loop(move |ctx| {
                mc_settlement_client::sync::sync_worker(
                    db_backend,
                    settlement_client,
                    chain_id,
                    l1_gas_provider,
                    gas_price_sync_disabled,
                    gas_price_poll,
                    mempool,
                    ctx,
                )
            });
        } else {
            tracing::error!("❗ Tried to start L1 Sync but no l1 endpoint was provided to the node on startup");
        }

        Ok(())
    }
}

impl<C, S> ServiceId for L1SyncService<C, S>
where
    C: Clone,
    S: Send + Stream<Item = Option<anyhow::Result<CommonMessagingEventData>>>,
{
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L1Sync.svc_id()
    }
}
