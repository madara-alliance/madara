use std::sync::Arc;

use crate::client::ClientTrait;
use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
use crate::messaging::CommonMessagingEventData;
use anyhow::Context;
use futures::Stream;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use mp_utils::trim_hash;
use serde::Deserialize;
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StateUpdate {
    pub block_number: u64,
    pub global_root: Felt,
    pub block_hash: Felt,
}

pub fn update_l1(
    backend: &MadaraBackend,
    state_update: StateUpdate,
    block_metrics: Arc<L1BlockMetrics>,
) -> anyhow::Result<()> {
    tracing::info!(
        "🔄 Updated L1 head #{} ({}) with state root ({})",
        state_update.block_number,
        trim_hash(&state_update.block_hash),
        trim_hash(&state_update.global_root)
    );

    block_metrics.l1_block_number.record(state_update.block_number, &[]);

    backend.write_last_confirmed_block(state_update.block_number).context("Setting l1 last confirmed block number")?;
    tracing::debug!("update_l1: wrote last confirmed block number");

    Ok(())
}

pub async fn state_update_worker<C, S>(
    backend: Arc<MadaraBackend>,
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, StreamType = S>>>,
    ctx: ServiceContext,
    l1_block_metrics: Arc<L1BlockMetrics>,
) -> anyhow::Result<()>
where
    S: Stream<Item = Option<Result<CommonMessagingEventData, SettlementClientError>>> + Send + 'static,
{
    // Clear L1 confirmed block at startup
    backend.clear_last_confirmed_block().context("Clearing l1 last confirmed block number")?;
    tracing::debug!("update_l1: cleared confirmed block number");

    tracing::info!("🚀 Subscribed to L1 state verification");
    // This does not seem to play well with anvil
    #[cfg(not(test))]
    {
        let initial_state = settlement_client.get_initial_state().await.context("Getting initial ethereum state")?;
        update_l1(&backend, initial_state, l1_block_metrics.clone())?;
    }

    settlement_client.listen_for_update_state_events(backend, ctx, l1_block_metrics.clone()).await?;
    anyhow::Ok(())
}
