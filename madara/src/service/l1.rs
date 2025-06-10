use crate::cli::l1::{L1SyncParams, MadaraSettlementLayer};
use alloy::primitives::Address;
use anyhow::Context;
use futures::Stream;
use mc_db::{DatabaseService, MadaraBackend};
use mc_mempool::{GasPriceProvider, Mempool};
use mc_settlement_client::client::SettlementClientTrait;
use mc_settlement_client::error::SettlementClientError;
use mc_settlement_client::eth::event::EthereumEventStream;
use mc_settlement_client::eth::{EthereumClient, EthereumClientConfig};
use mc_settlement_client::gas_price::L1BlockMetrics;
use mc_settlement_client::messaging::L1ToL2MessagingEventData;
use mc_settlement_client::starknet::event::StarknetEventStream;
use mc_settlement_client::starknet::{StarknetClient, StarknetClientConfig};
use mc_settlement_client::state_update::L1HeadSender;
use mc_settlement_client::sync::SyncWorkerConfig;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use starknet_core::types::Felt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

// Configuration struct to group related parameters
pub struct L1SyncConfig<'a> {
    pub db: &'a DatabaseService,
    pub l1_gas_provider: GasPriceProvider,
    pub l1_core_address: String,
    pub authority: bool,
    pub devnet: bool,
    pub mempool: Arc<Mempool>,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_snd: L1HeadSender,
}

pub struct L1SyncService<C: 'static, S: 'static>
where
    C: Clone,
    S: Send + Stream<Item = Result<L1ToL2MessagingEventData, SettlementClientError>>,
{
    db_backend: Arc<MadaraBackend>,
    l1_head_snd: Option<L1HeadSender>,
    settlement_client: Option<Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>>,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll: Duration,
    mempool: Arc<Mempool>,
    l1_block_metrics: Arc<L1BlockMetrics>,
}

pub type EthereumSyncService = L1SyncService<EthereumClientConfig, EthereumEventStream>;
pub type StarknetSyncService = L1SyncService<StarknetClientConfig, StarknetEventStream>;

// Implementation for Ethereum
impl EthereumSyncService {
    pub async fn new(config: &L1SyncParams, sync_config: L1SyncConfig<'_>) -> anyhow::Result<Self> {
        let settlement_client = {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                let core_address = Address::from_str(sync_config.l1_core_address.as_str())?;
                let client = EthereumClient::new(EthereumClientConfig {
                    url: l1_rpc_url.clone(),
                    l1_core_address: core_address,
                })
                .await
                .context("Creating ethereum client")?;

                Some(into_trait_arc(client))
            } else {
                None
            }
        };

        Self::create_service(config, sync_config, settlement_client).await
    }
}

// Implementation for Starknet
impl StarknetSyncService {
    pub async fn new(config: &L1SyncParams, sync_config: L1SyncConfig<'_>) -> anyhow::Result<Self> {
        let settlement_client = {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                let core_address = Felt::from_str(sync_config.l1_core_address.as_str())?;
                let client = StarknetClient::new(StarknetClientConfig {
                    url: l1_rpc_url.clone(),
                    l2_contract_address: core_address,
                })
                .await
                .context("Creating starknet client")?;

                Some(into_trait_arc(client))
            } else {
                None
            }
        };

        Self::create_service(config, sync_config, settlement_client).await
    }
}

// Shared implementation for both services
impl<C: Clone, S> L1SyncService<C, S>
where
    C: Clone + 'static,
    S: Send + Stream<Item = Result<L1ToL2MessagingEventData, SettlementClientError>> + 'static,
{
    async fn create_service(
        config: &L1SyncParams,
        sync_config: L1SyncConfig<'_>,
        settlement_client: Option<Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>>,
    ) -> anyhow::Result<Self> {
        let gas_price_sync_enabled = sync_config.authority
            && !sync_config.devnet
            && (config.gas_price.is_none() || config.blob_gas_price.is_none());
        let gas_price_poll = config.gas_price_poll;

        if gas_price_sync_enabled {
            let settlement_client =
                settlement_client.clone().context("L1 gas prices require the service to be enabled...")?;
            tracing::info!("⏳ Getting initial L1 gas prices");
            mc_settlement_client::gas_price::gas_price_worker_once(
                settlement_client,
                &sync_config.l1_gas_provider,
                gas_price_poll,
                sync_config.l1_block_metrics.clone(),
            )
            .await
            .context("Getting initial gas prices")?;
        }

        Ok(Self {
            db_backend: Arc::clone(sync_config.db.backend()),
            settlement_client,
            l1_gas_provider: sync_config.l1_gas_provider,
            gas_price_sync_disabled: !gas_price_sync_enabled,
            gas_price_poll,
            mempool: sync_config.mempool,
            l1_block_metrics: sync_config.l1_block_metrics,
            l1_head_snd: Some(sync_config.l1_head_snd),
        })
    }

    // Factory method to create the appropriate service
    pub async fn create(config: &L1SyncParams, sync_config: L1SyncConfig<'_>) -> anyhow::Result<Box<dyn Service>> {
        match config.settlement_layer {
            MadaraSettlementLayer::Eth => Ok(Box::new(EthereumSyncService::new(config, sync_config).await?)),
            MadaraSettlementLayer::Starknet => Ok(Box::new(StarknetSyncService::new(config, sync_config).await?)),
        }
    }
}

#[async_trait::async_trait]
impl<C, S> Service for L1SyncService<C, S>
where
    C: Clone,
    S: Send + Stream<Item = Result<L1ToL2MessagingEventData, SettlementClientError>>,
{
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if let Some(settlement_client) = &self.settlement_client {
            let db_backend = Arc::clone(&self.db_backend);
            let settlement_client = Arc::clone(settlement_client);
            let l1_gas_provider = self.l1_gas_provider.clone();
            let gas_price_sync_disabled = self.gas_price_sync_disabled;
            let gas_price_poll = self.gas_price_poll;
            let mempool = Arc::clone(&self.mempool);
            let l1_block_metrics = self.l1_block_metrics.clone();
            let l1_head_sender = self.l1_head_snd.take().expect("Service already starteds");

            runner.service_loop(move |ctx| {
                mc_settlement_client::sync::sync_worker(SyncWorkerConfig {
                    backend: db_backend,
                    settlement_client,
                    l1_gas_provider,
                    gas_price_sync_disabled,
                    gas_price_poll_ms: gas_price_poll,
                    mempool,
                    l1_head_sender,
                    ctx,
                    l1_block_metrics,
                })
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
    S: Send + Stream<Item = Result<L1ToL2MessagingEventData, SettlementClientError>>,
{
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L1Sync.svc_id()
    }
}

fn into_trait_arc<T, C, S>(client: T) -> Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>
where
    T: SettlementClientTrait<Config = C, StreamType = S> + 'static,
{
    Arc::new(client)
}
