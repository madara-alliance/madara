use crate::cli::l1::{L1SyncParams, MadaraSettlementLayer};
use anyhow::Context;
use mc_db::MadaraBackend;
use mc_mempool::GasPriceProvider;
use mc_settlement_client::state_update::L1HeadSender;
use mc_settlement_client::sync::SyncWorkerConfig;
use mc_settlement_client::{gas_price::L1BlockMetrics, SettlementClient};
use mc_settlement_client::{L1SyncDisabledClient, L1ClientImpl};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

// Configuration struct to group related parameters
pub struct L1SyncConfig {
    pub l1_gas_provider: GasPriceProvider,
    pub l1_core_address: String,
    pub authority: bool,
    pub devnet: bool,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_snd: L1HeadSender,
}

pub struct L1SyncService {
    sync_worker_config: Option<SyncWorkerConfig>,
    client: Option<Arc<L1ClientImpl>>,
}

impl L1SyncService {
    pub async fn new(
        config: &L1SyncParams,
        backend: Arc<MadaraBackend>,
        sync_config: L1SyncConfig,
    ) -> anyhow::Result<Self> {
        if config.l1_sync_disabled {
            return Ok(Self { sync_worker_config: None, client: None });
        }
        let endpoint = config.l1_endpoint.clone().context("Missing l1_endpoint")?;
        let client = match config.settlement_layer {
            MadaraSettlementLayer::Eth => L1ClientImpl::new_ethereum(backend, endpoint, sync_config.l1_core_address)
                .await
                .context("Starting ethereum core contract client")?,
            MadaraSettlementLayer::Starknet => {
                L1ClientImpl::new_starknet(backend, endpoint, sync_config.l1_core_address)
                    .await
                    .context("Starting starknet core contract client")?
            }
        };

        let gas_price_sync_enabled = sync_config.authority
            && !sync_config.devnet
            && (config.gas_price.is_none() || config.blob_gas_price.is_none());
        let gas_price_poll = config.gas_price_poll;

        if gas_price_sync_enabled {
            tracing::info!("⏳ Getting initial L1 gas prices");
            client
                .gas_price_worker_once(
                    &sync_config.l1_gas_provider,
                    gas_price_poll,
                    sync_config.l1_block_metrics.clone(),
                )
                .await
                .context("Getting initial gas prices")?;
        }

        Ok(Self {
            client: Some(client.into()),
            sync_worker_config: Some(SyncWorkerConfig {
                l1_gas_provider: sync_config.l1_gas_provider,
                gas_price_sync_disabled: !gas_price_sync_enabled,
                gas_price_poll,
                l1_head_sender: sync_config.l1_head_snd,
                l1_block_metrics: sync_config.l1_block_metrics,
            }),
        })
    }

    pub fn client(&self) -> Arc<dyn SettlementClient> {
        if let Some(client) = self.client.clone() {
            client
        } else {
            Arc::new(L1SyncDisabledClient)
        }
    }
}

#[async_trait::async_trait]
impl Service for L1SyncService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if let Some((config, client)) = self.sync_worker_config.take().zip(self.client.clone()) {
            runner.service_loop(move |ctx| client.run_sync_worker(ctx, config));
        } else {
            tracing::debug!("l1 sync is disabled");
        }

        Ok(())
    }
}

impl ServiceId for L1SyncService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L1Sync.svc_id()
    }
}
