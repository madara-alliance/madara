use crate::client::SettlementLayerProvider;
use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
use crate::{RECONNECT_BASE_DELAY, RECONNECT_MAX_DELAY};
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use mp_utils::trim_hash;
use serde::Deserialize;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StateUpdate {
    pub block_number: Option<u64>,
    pub global_root: Felt,
    pub block_hash: Felt,
}

pub type L1HeadReceiver = tokio::sync::watch::Receiver<Option<StateUpdate>>;
pub type L1HeadSender = tokio::sync::watch::Sender<Option<StateUpdate>>;

pub struct StateUpdateWorker {
    block_metrics: Arc<L1BlockMetrics>,
    backend: Arc<MadaraBackend>,
    l1_head_sender: L1HeadSender,
}

impl StateUpdateWorker {
    pub fn update_state(&self, state_update: StateUpdate) -> Result<(), SettlementClientError> {
        let block_info = match state_update.block_number {
            Some(num) => {
                self.block_metrics.l1_block_number.record(num, &[]);
                self.backend.set_latest_l1_confirmed(Some(num)).map_err(|e| {
                    SettlementClientError::DatabaseError(format!("Failed to write last confirmed block: {}", e))
                })?;
                tracing::debug!("Wrote last confirmed block number: {}", num);
                format!("#{}", num)
            }
            None => {
                tracing::warn!("No valid block number received from L1");
                "no block number".to_string()
            }
        };

        tracing::info!(
            "🔄 L1 State Update: {} | BlockHash: {} | GlobalRoot: {}",
            block_info,
            trim_hash(&state_update.block_hash),
            trim_hash(&state_update.global_root)
        );

        self.l1_head_sender.send_modify(|s| *s = Some(state_update.clone()));

        Ok(())
    }
}

pub async fn state_update_worker(
    backend: Arc<MadaraBackend>,
    settlement_client: Arc<dyn SettlementLayerProvider>,
    mut ctx: ServiceContext,
    l1_head_sender: L1HeadSender,
    block_metrics: Arc<L1BlockMetrics>,
) -> Result<(), SettlementClientError> {
    let state = StateUpdateWorker {
        block_metrics: block_metrics.clone(),
        backend: backend.clone(),
        l1_head_sender: l1_head_sender.clone(),
    };

    // Clear L1 confirmed block at startup
    // TODO: remove this
    state.backend.set_latest_l1_confirmed(None).map_err(|e| {
        SettlementClientError::DatabaseError(format!("Failed to clear last confirmed block at startup: {}", e))
    })?;
    tracing::debug!("update_l1: cleared confirmed block number");

    let mut reconnect_delay = RECONNECT_BASE_DELAY;

    loop {
        match ctx
            .run_until_cancelled(try_sync_once(
                &settlement_client,
                block_metrics.clone(),
                backend.clone(),
                l1_head_sender.clone(),
                ctx.clone(),
            ))
            .await
        {
            None => return Ok(()),         // Service shutdown
            Some(Ok(())) => return Ok(()), // Clean exit
            Some(Err(e)) if e.is_recoverable() => {
                tracing::warn!("L1 state sync failed: {e:#}, reconnecting in {reconnect_delay:?}");
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, RECONNECT_MAX_DELAY);
            }
            Some(Err(e)) => return Err(e), // Non-recoverable
        }
    }
}

/// Runs a single state sync session. Returns when the stream ends or an error occurs.
async fn try_sync_once(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    block_metrics: Arc<L1BlockMetrics>,
    backend: Arc<MadaraBackend>,
    l1_head_sender: L1HeadSender,
    ctx: ServiceContext,
) -> Result<(), SettlementClientError> {
    let state = StateUpdateWorker { block_metrics, backend, l1_head_sender };
    let initial_state = settlement_client.get_current_core_contract_state().await?;

    tracing::info!("Subscribed to L1 state verification");
    state.update_state(initial_state)?;

    settlement_client.listen_for_update_state_events(ctx, state).await
}
