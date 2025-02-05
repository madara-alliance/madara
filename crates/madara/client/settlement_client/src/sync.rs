use crate::client::ClientTrait;
use crate::error::SettlementClientError;
use crate::gas_price::{gas_price_worker, L1BlockMetrics};
use crate::messaging::{sync, CommonMessagingEventData};
use crate::state_update::state_update_worker;
use futures::Stream;
use mc_db::MadaraBackend;
use mc_mempool::{GasPriceProvider, Mempool};
use mp_utils::service::ServiceContext;
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
pub async fn sync_worker<C: 'static, S>(
    backend: Arc<MadaraBackend>,
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, StreamType = S>>>,
    chain_id: ChainId,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: Duration,
    mempool: Arc<Mempool>,
    ctx: ServiceContext,
    l1_block_metrics: Arc<L1BlockMetrics>,
) -> anyhow::Result<()>
where
    S: Stream<Item = Option<Result<CommonMessagingEventData, SettlementClientError>>> + Send + 'static,
{
    let mut join_set = tokio::task::JoinSet::new();

    join_set.spawn(state_update_worker(
        Arc::clone(&backend),
        settlement_client.clone(),
        ctx.clone(),
        l1_block_metrics.clone(),
    ));

    join_set.spawn(sync(settlement_client.clone(), Arc::clone(&backend), chain_id, mempool, ctx.clone()));

    if !gas_price_sync_disabled {
        join_set.spawn(gas_price_worker(
            settlement_client.clone(),
            l1_gas_provider,
            gas_price_poll_ms,
            ctx.clone(),
            l1_block_metrics,
        ));
    }

    while let Some(res) = join_set.join_next().await {
        res??;
    }

    Ok(())
}
