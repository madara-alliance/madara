use crate::client::ClientTrait;
use crate::gas_price::gas_price_worker;
use crate::messaging::sync::sync;
use crate::state_update::state_update_worker;
use mc_db::MadaraBackend;
use mc_mempool::{GasPriceProvider, Mempool};
use mp_utils::service::ServiceContext;
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
pub async fn sync_worker<C: 'static, E: 'static>(
    backend: Arc<MadaraBackend>,
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, EventStruct = E>>>,
    chain_id: ChainId,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: Duration,
    mempool: Arc<Mempool>,
    ctx: ServiceContext,
) -> anyhow::Result<()> {
    let mut join_set = tokio::task::JoinSet::new();

    join_set.spawn(state_update_worker(Arc::clone(&backend), settlement_client.clone(), ctx.clone()));

    join_set.spawn(sync(settlement_client.clone(), Arc::clone(&backend), chain_id, mempool, ctx.clone()));

    if !gas_price_sync_disabled {
        join_set.spawn(gas_price_worker(settlement_client.clone(), l1_gas_provider, gas_price_poll_ms, ctx.clone()));
    }

    while let Some(res) = join_set.join_next().await {
        res??;
    }

    Ok(())
}
