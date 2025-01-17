use crate::cli::l1::L1SyncParams;
use alloy::primitives::Address;
use anyhow::Context;
use mc_db::{DatabaseService, MadaraBackend};
use mc_eth::client::{EthereumClient, L1BlockMetrics};
use mc_mempool::{GasPriceProvider, Mempool};
use mp_block::H160;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct L1SyncService {
    db_backend: Arc<MadaraBackend>,
    eth_client: Option<Arc<EthereumClient>>,
    l1_gas_provider: GasPriceProvider,
    chain_id: ChainId,
    gas_price_sync_disabled: bool,
    gas_price_poll: Duration,
    mempool: Arc<Mempool>,
}

impl L1SyncService {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: &L1SyncParams,
        db: &DatabaseService,
        l1_gas_provider: GasPriceProvider,
        chain_id: ChainId,
        l1_core_address: H160,
        authority: bool,
        devnet: bool,
        mempool: Arc<Mempool>,
    ) -> anyhow::Result<Self> {
        let eth_client = if !config.l1_sync_disabled && (config.l1_endpoint.is_some() || !devnet) {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                let core_address = Address::from_slice(l1_core_address.as_bytes());
                let l1_block_metrics = L1BlockMetrics::register().expect("Registering metrics");
                let client = EthereumClient::new(l1_rpc_url.clone(), core_address, l1_block_metrics)
                    .await
                    .context("Creating ethereum client")?;

                Some(Arc::new(client))
            } else {
                anyhow::bail!(
                    "No Ethereum endpoint provided. You need to provide one using --l1-endpoint <RPC URL> in order to verify the synced state or disable the l1 watcher using --no-l1-sync."
                );
            }
        } else {
            None
        };

        // Note: gas price should be synced in case the madara is running in sequencer mode,
        // we haven't set any fix price for the gas, hence gas price should be none
        let gas_price_sync_enabled =
            authority && !devnet && (config.gas_price.is_none() || config.blob_gas_price.is_none());
        let gas_price_poll = config.gas_price_poll;

        if gas_price_sync_enabled {
            let eth_client = eth_client
                .clone()
                .context("L1 gas prices require the ethereum service to be enabled. Either disable gas prices syncing using `--gas-price 0`, or disable L1 sync using the `--no-l1-sync` argument.")?;
            // running at-least once before the block production service
            tracing::info!("⏳ Getting initial L1 gas prices");
            mc_eth::l1_gas_price::gas_price_worker_once(&eth_client, &l1_gas_provider, gas_price_poll)
                .await
                .context("Getting initial ethereum gas prices")?;
        }

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            eth_client,
            l1_gas_provider,
            chain_id,
            gas_price_sync_disabled: !gas_price_sync_enabled,
            gas_price_poll,
            mempool,
        })
    }
}

#[async_trait::async_trait]
impl Service for L1SyncService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let L1SyncService {
            db_backend,
            l1_gas_provider,
            chain_id,
            gas_price_sync_disabled,
            gas_price_poll,
            mempool,
            ..
        } = self.clone();

        if let Some(eth_client) = &self.eth_client {
            // enabled

            let eth_client = Arc::clone(eth_client);
            runner.service_loop(move |ctx| {
                mc_eth::sync::l1_sync_worker(
                    db_backend,
                    eth_client,
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

impl ServiceId for L1SyncService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L1Sync.svc_id()
    }
}
