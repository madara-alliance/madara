use crate::client::{ClientTrait, ClientType};
use crate::gas_price::gas_price_worker;
use crate::l1_messaging::sync;
use crate::state_update::state_update_worker;
use mc_db::MadaraBackend;
use mc_mempool::{GasPriceProvider, Mempool};
use mp_utils::service::ServiceContext;
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

#[allow(clippy::too_many_arguments)]
pub async fn sync_worker<C: 'static, P: 'static>(
    backend: Arc<MadaraBackend>,
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, Provider = P>>>,
    chain_id: ChainId,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: Duration,
    mempool: Arc<Mempool>,
    ctx: ServiceContext,
) -> anyhow::Result<()> {
    let mut join_set = tokio::task::JoinSet::new();

    join_set.spawn(state_update_worker(Arc::clone(&backend), settlement_client.clone(), ctx.clone()));

    match settlement_client.get_client_type() {
        ClientType::ETH => {
            join_set.spawn(sync(Arc::clone(&backend), settlement_client.clone(), chain_id, mempool, ctx.clone()));
        }
        _ => {
            warn!("⚠️ Provided client type does not implement the messaging sync function. Continuing....")
        }
    }

    if !gas_price_sync_disabled {
        join_set.spawn(gas_price_worker(settlement_client.clone(), l1_gas_provider, gas_price_poll_ms, ctx.clone()));
    }

    while let Some(res) = join_set.join_next().await {
        res??;
    }

    Ok(())
}
