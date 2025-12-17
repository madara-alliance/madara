use crate::client::SettlementLayerProvider;
use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use mp_utils::trim_hash;
use serde::Deserialize;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::time::Duration;

/// Base delay for reconnection attempts after stream failure.
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);

/// Maximum delay between reconnection attempts (exponential backoff cap).
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StateUpdate {
    pub block_number: Option<u64>,
    pub global_root: Felt,
    pub block_hash: Felt,
}

pub type L1HeadReceiver = tokio::sync::watch::Receiver<Option<StateUpdate>>;
pub type L1HeadSender = tokio::sync::watch::Sender<Option<StateUpdate>>;

#[derive(Clone)]
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
    let state = StateUpdateWorker { block_metrics, backend, l1_head_sender };

    // Clear L1 confirmed block at startup
    // TODO: remove this
    state.backend.set_latest_l1_confirmed(None).map_err(|e| {
        SettlementClientError::DatabaseError(format!("Failed to clear last confirmed block at startup: {}", e))
    })?;
    tracing::debug!("update_l1: cleared confirmed block number");

    let mut reconnect_delay = RECONNECT_BASE_DELAY;

    // Outer loop for reconnection on stream/connection failure
    loop {
        // Get initial state on each reconnection attempt
        let initial_state = match ctx.run_until_cancelled(settlement_client.get_current_core_contract_state()).await {
            Some(Ok(state)) => {
                // Reset backoff on successful state fetch
                reconnect_delay = RECONNECT_BASE_DELAY;
                state
            }
            Some(Err(e)) => {
                tracing::warn!("Failed to get initial L1 state: {e:#}, retrying in {:?}", reconnect_delay);
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, RECONNECT_MAX_DELAY);
                continue;
            }
            None => return Ok(()), // Service shutdown while getting initial state
        };

        tracing::info!("Subscribed to L1 state verification");

        if let Err(e) = state.update_state(initial_state) {
            tracing::warn!("Failed to update L1 with initial state: {e:#}");
            // Continue to try listening anyway - the state might still be valid
        }

        // Listen for state update events
        match settlement_client.listen_for_update_state_events(ctx.clone(), state.clone()).await {
            Ok(()) => return Ok(()), // Clean shutdown (context cancelled)
            Err(e) if is_stream_ended_error(&e) => {
                // Check if context was cancelled during the listen call
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tracing::warn!("L1 state update stream ended: {e:#}, reconnecting in {:?}", reconnect_delay);
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, RECONNECT_MAX_DELAY);
                // Continue outer loop to reconnect
            }
            Err(e) => {
                // Non-recoverable error (e.g., contract issues)
                return Err(SettlementClientError::StateEventListener(format!(
                    "Failed to listen for update state events: {e:#}"
                )));
            }
        }
    }
}

/// Check if the error indicates a stream/connection ended (recoverable)
fn is_stream_ended_error(e: &SettlementClientError) -> bool {
    matches!(e, SettlementClientError::StreamProcessing(_) | SettlementClientError::StateEventListener(_))
}
