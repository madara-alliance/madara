use crate::cli::l1::L1SyncParams;
use alloy::primitives::Address;
use anyhow::Context;
use dc_db::{DatabaseService, DeoxysBackend};
use dc_eth::client::EthereumClient;
// use dc_eth::l1_gas_price::L1GasPrices;
// use dc_eth::l1_gas_price::GasPriceProvider;
use dc_mempool::L1DataProvider;
use dc_metrics::MetricsRegistry;
use dp_block::header::GasPrices;
use dp_convert::ToFelt;
use dp_utils::service::Service;
use futures::lock::Mutex;
use primitive_types::H160;
use starknet_api::core::ChainId;
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct L1SyncService {
    db_backend: Arc<DeoxysBackend>,
    eth_client: EthereumClient,
    l1_data_provider: Arc<dyn L1DataProvider>,
    chain_id: ChainId,
    gas_price_sync_disabled: bool,
}

impl L1SyncService {
    pub async fn new(
        config: &L1SyncParams,
        db: &DatabaseService,
        metrics_handle: MetricsRegistry,
        l1_data_provider: Arc<dyn L1DataProvider>,
        chain_id: ChainId,
        l1_core_address: H160,
    ) -> anyhow::Result<Self> {
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

        let core_address = Address::from_slice(l1_core_address.as_bytes());
        let eth_client = EthereumClient::new(l1_endpoint.unwrap(), core_address, metrics_handle)
            .await
            .context("Creating ethereum client")?;
        let gas_price_sync_disabled = config.gas_price_sync_disabled;

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            eth_client,
            l1_data_provider,
            chain_id,
            gas_price_sync_disabled,
        })
    }
}

#[async_trait::async_trait]
impl Service for L1SyncService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        let L1SyncService { eth_client, l1_data_provider, chain_id, gas_price_sync_disabled, .. } = self.clone();

        let db_backend = Arc::clone(&self.db_backend);

        join_set.spawn(async move {
            dc_eth::sync::l1_sync_worker(
                &db_backend,
                &eth_client,
                chain_id.to_felt(),
                l1_data_provider,
                gas_price_sync_disabled,
            )
            .await
        });

        Ok(())
    }
}
