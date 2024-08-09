use crate::cli::l1::L1SyncParams;
use alloy::primitives::Address;
use anyhow::Context;
use dc_db::{DatabaseService, DeoxysBackend};
use dc_eth::client::EthereumClient;
use dc_mempool::L1DataProvider;
use dc_metrics::MetricsRegistry;
use dp_convert::ToFelt;
use dp_utils::service::Service;
use primitive_types::H160;
use starknet_api::core::ChainId;
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct L1SyncService {
    db_backend: Arc<DeoxysBackend>,
    eth_client: Option<EthereumClient>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    chain_id: ChainId,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: u64,
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
        let eth_client = if !config.sync_l1_disabled {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                let core_address = Address::from_slice(l1_core_address.as_bytes());
                Some(
                    EthereumClient::new(l1_rpc_url.clone(), core_address, metrics_handle)
                        .await
                        .context("Creating ethereum client")?,
                )
            } else {
                anyhow::bail!(
                    "No Ethereum endpoint provided. You need to provide one using --l1-endpoint <RPC URL> in order to verify the synced state or disable the l1 watcher using --no-l1-sync."
                );
            }
        } else {
            None
        };

        let gas_price_sync_disabled = config.gas_price_sync_disabled;
        let gas_price_poll_ms = config.gas_price_poll_ms;

        if !gas_price_sync_disabled {
            let eth_client = eth_client.clone().ok_or_else(|| {
                anyhow::anyhow!("EthereumClient is required to start the l1 sync service but not provided.")
            })?;
            // running at-least once before the block production service
            let _ = futures::executor::block_on(dc_eth::l1_gas_price::gas_price_worker(
                &eth_client,
                Arc::clone(&l1_data_provider),
                false,
                gas_price_poll_ms,
            ));
        }

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            eth_client,
            l1_data_provider,
            chain_id,
            gas_price_sync_disabled,
            gas_price_poll_ms,
        })
    }
}

#[async_trait::async_trait]
impl Service for L1SyncService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        let L1SyncService {
            eth_client, l1_data_provider, chain_id, gas_price_sync_disabled, gas_price_poll_ms, ..
        } = self.clone();

        let db_backend = Arc::clone(&self.db_backend);

        let eth_client = eth_client.ok_or_else(|| {
            anyhow::anyhow!("EthereumClient is required to start the l1 sync service but not provided.")
        })?;

        join_set.spawn(async move {
            dc_eth::sync::l1_sync_worker(
                &db_backend,
                &eth_client,
                chain_id.to_felt(),
                l1_data_provider,
                gas_price_sync_disabled,
                gas_price_poll_ms,
            )
            .await
        });

        Ok(())
    }
}
