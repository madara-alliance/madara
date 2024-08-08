use crate::cli::l1::L1SyncParams;
use crate::cli::SyncParams;
use alloy::primitives::Address;
use anyhow::Context;
use dc_db::db_metrics::DbMetrics;
use dc_db::{DatabaseService, DeoxysBackend};
use dc_eth::client::EthereumClient;
use dc_eth::l1_gas_price::L1GasPrices;
use dc_metrics::MetricsRegistry;
use dc_sync::fetch::fetchers::FetchConfig;
use dc_sync::metrics::block_metrics::BlockMetrics;
use dc_telemetry::TelemetryHandle;
use dp_convert::ToFelt;
use dp_utils::service::Service;
use futures::lock::Mutex;
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct L1SyncService {
    db_backend: Arc<DeoxysBackend>,
    // backup_every_n_blocks: Option<u64>,
    eth_client: EthereumClient,
    l1_gas_prices: Arc<Mutex<L1GasPrices>>,
    chain_id: ChainId, // starting_block: Option<u64>,
                       // block_metrics: BlockMetrics,
                       // db_metrics: DbMetrics,
                       // start_params: Option<TelemetryHandle>,
                       // disabled: bool,
                       // pending_block_poll_interval: Duration,
}

impl L1SyncService {
    pub async fn new(
        config: &L1SyncParams,
        db: &DatabaseService,
        metrics_handle: MetricsRegistry,
        telemetry: TelemetryHandle,
        l1_gas_prices: Arc<Mutex<L1GasPrices>>,
    ) -> anyhow::Result<Self> {
        // TODO: create l1 metrics here
        // let block_metrics = BlockMetrics::register(&metrics_handle)?;
        // let db_metrics = DbMetrics::register(&metrics_handle)?;
        let chain_id = config.block_fetch_config();

        let l1_endpoint = if !config.sync_l1_disabled {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                Some(l1_rpc_url.clone())
            } else {
                return Err(anyhow::anyhow!(
                    "❗ No L1 endpoint provided. You must provide one in order to verify the synced state."
                ));
            }
        } else {
            None
        };

        let core_address = Address::from_slice(config.network.l1_core_address().as_bytes());
        let eth_client = EthereumClient::new(l1_endpoint.unwrap(), core_address, metrics_handle)
            .await
            .context("Creating ethereum client")?;

        Ok(Self { db_backend: Arc::clone(db.backend()), eth_client, l1_gas_prices, chain_id })
    }
}

#[async_trait::async_trait]
impl Service for L1SyncService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        // if self.disabled {
        //     return Ok(());
        // }
        let L1SyncService { eth_client, l1_gas_prices, chain_id, .. } = self.clone();
        // let telemetry = self.start_params.take().context("Service already started")?;

        let db_backend = Arc::clone(&self.db_backend);

        join_set.spawn(
            async move { dc_eth::sync::sync(&db_backend, &eth_client, chain_id.to_felt(), l1_gas_prices).await },
        );

        Ok(())
    }
}
