use crate::client::SettlementClientTrait;
use crate::error::SettlementClientError;
use crate::gas_price::{gas_price_worker, GasPriceProviderConfig, L1BlockMetrics};
use crate::messaging::{sync, L1toL2MessagingEventData};
use crate::state_update::{state_update_worker, L1HeadSender};
use futures::Stream;
use mc_db::MadaraBackend;
use mc_mempool::Mempool;
use mp_utils::service::ServiceContext;
use std::sync::Arc;

pub struct SyncWorkerConfig<C: 'static, S> {
    pub backend: Arc<MadaraBackend>,
    pub settlement_client: Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>,
    pub gas_provider_config: Option<GasPriceProviderConfig>,
    pub mempool: Arc<Mempool>,
    pub ctx: ServiceContext,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_sender: L1HeadSender,
}

pub async fn sync_worker<C: 'static, S>(config: SyncWorkerConfig<C, S>) -> anyhow::Result<()>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    let mut join_set = tokio::task::JoinSet::new();

    join_set.spawn(state_update_worker(
        config.settlement_client.clone(),
        Arc::clone(&config.backend),
        config.ctx.clone(),
        config.l1_head_sender,
        config.l1_block_metrics.clone(),
    ));

    join_set.spawn(sync(
        config.settlement_client.clone(),
        Arc::clone(&config.backend),
        config.mempool,
        config.ctx.clone(),
    ));

    if let Some(gas_provider_config) = config.gas_provider_config {
        join_set.spawn(gas_price_worker(
            config.settlement_client.clone(),
            Arc::clone(&config.backend),
            config.ctx.clone(),
            gas_provider_config,
            config.l1_block_metrics,
        ));
    }

    while let Some(res) = join_set.join_next().await {
        res??;
    }

    Ok(())
}
