use crate::client::EthereumClient;
use crate::l1_gas_price::gas_price_worker;
use crate::l1_messaging::sync;
use crate::state_update::{state_update_worker, L1HeadSender};
use mc_db::MadaraBackend;
use mc_mempool::GasPriceProvider;
use mc_submit_tx::SubmitL1HandlerTransaction;
use mp_utils::service::ServiceContext;
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
pub async fn l1_sync_worker(
    backend: Arc<MadaraBackend>,
    eth_client: Arc<EthereumClient>,
    chain_id: ChainId,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: Duration,
    submit_tx: Arc<dyn SubmitL1HandlerTransaction>,
    l1_head_sender: L1HeadSender,
    ctx: ServiceContext,
) -> anyhow::Result<()> {
    let mut join_set = tokio::task::JoinSet::new();

    join_set.spawn(state_update_worker(Arc::clone(&backend), Arc::clone(&eth_client), ctx.clone(), l1_head_sender));
    join_set.spawn(sync(Arc::clone(&backend), Arc::clone(&eth_client), chain_id, submit_tx, ctx.clone()));

    if !gas_price_sync_disabled {
        join_set.spawn(gas_price_worker(Arc::clone(&eth_client), l1_gas_provider, gas_price_poll_ms, ctx.clone()));
    }

    while let Some(res) = join_set.join_next().await {
        res??;
    }

    Ok(())
}
