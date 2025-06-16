use crate::client::SettlementLayerProvider;
use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
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
                self.backend.write_last_confirmed_block(num).map_err(|e| {
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
    state.backend.clear_last_confirmed_block().map_err(|e| {
        SettlementClientError::DatabaseError(format!("Failed to clear last confirmed block at startup: {}", e))
    })?;
    tracing::debug!("update_l1: cleared confirmed block number");

    let Some(initial_state) = ctx
        .run_until_cancelled(settlement_client.get_current_core_contract_state())
        .await
        .transpose()
        .map_err(|e| SettlementClientError::StateInitialization(format!("Failed to get initial state: {e:#}")))?
    else {
        return Ok(()); // Service shutdown while getting initial state
    };

    tracing::info!("🚀 Subscribed to L1 state verification");

    state
        .update_state(initial_state)
        .map_err(|e| SettlementClientError::StateUpdate(format!("Failed to update L1 with initial state: {e:#}")))?;

    settlement_client.listen_for_update_state_events(ctx.clone(), state).await.map_err(|e| {
        SettlementClientError::StateEventListener(format!("Failed to listen for update state events: {e:#}"))
    })
}
