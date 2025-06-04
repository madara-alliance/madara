use crate::client::SettlementClientTrait;
use crate::gas_price::{gas_price_worker, L1BlockMetrics};
use crate::messaging::sync;
use crate::state_update::{state_update_worker, L1HeadSender};
use mc_db::MadaraBackend;
use mc_mempool::{GasPriceProvider, Mempool};
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use std::time::Duration;

pub struct SyncWorkerConfig {
    pub backend: Arc<MadaraBackend>,
    pub settlement_client: Arc<dyn SettlementClientTrait>,
    pub l1_gas_provider: GasPriceProvider,
    pub gas_price_sync_disabled: bool,
    pub gas_price_poll_ms: Duration,
    pub mempool: Arc<Mempool>,
    pub ctx: ServiceContext,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_sender: L1HeadSender,
}

pub async fn sync_worker(config: SyncWorkerConfig) -> anyhow::Result<()> {
    let mut join_set = tokio::task::JoinSet::new();

    join_set.spawn(state_update_worker(
        Arc::clone(&config.backend),
        config.settlement_client.clone(),
        config.ctx.clone(),
        config.l1_head_sender,
        config.l1_block_metrics.clone(),
    ));

    join_set.spawn(sync(config.settlement_client.clone(), Arc::clone(&config.backend), config.ctx.clone()));

    if !config.gas_price_sync_disabled {
        join_set.spawn(gas_price_worker(
            config.settlement_client.clone(),
            config.l1_gas_provider,
            config.gas_price_poll_ms,
            config.ctx.clone(),
            config.l1_block_metrics,
        ));
    }

    while let Some(res) = join_set.join_next().await {
        res??;
    }

    Ok(())
}
