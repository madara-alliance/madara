use crate::client::SettlementLayerProvider;
use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
use crate::{RECONNECT_BASE_DELAY, RECONNECT_MAX_DELAY};
use mc_db::MadaraBackend;
use mp_utils::service::ServiceContext;
use mp_utils::trim_hash;
use serde::Deserialize;
use starknet_types_core::felt::Felt;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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

    let persisted_l1_cursor = state.backend.latest_l1_confirmed_block_n();
    let preserve_l1_cursor = std::env::var_os("MADARA_DEBUG_PRESERVE_L1_CURSOR_ON_STARTUP").is_some();
    tracing::info!(
        target: "startup_recovery",
        phase = "l1_cursor_loaded",
        ?persisted_l1_cursor,
        preserve_l1_cursor,
        "phase=l1_cursor_loaded persisted_l1_cursor={persisted_l1_cursor:?} preserve_l1_cursor={preserve_l1_cursor}"
    );

    if preserve_l1_cursor {
        tracing::warn!(
            target: "startup_recovery",
            phase = "l1_cursor_preserved_debug_override",
            ?persisted_l1_cursor,
            "phase=l1_cursor_preserved_debug_override persisted_l1_cursor={persisted_l1_cursor:?}"
        );
    } else {
        // Clear L1 confirmed block at startup
        // TODO: remove this
        state.backend.set_latest_l1_confirmed(None).map_err(|e| {
            SettlementClientError::DatabaseError(format!("Failed to clear last confirmed block at startup: {}", e))
        })?;
        tracing::info!(
            target: "startup_recovery",
            phase = "l1_cursor_cleared",
            ?persisted_l1_cursor,
            "phase=l1_cursor_cleared persisted_l1_cursor={persisted_l1_cursor:?}"
        );
    }

    if let Some(delay_ms) = std::env::var("MADARA_DEBUG_L1_CURSOR_REPOPULATE_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|delay_ms| *delay_ms > 0)
    {
        tracing::warn!(
            target: "startup_recovery",
            phase = "l1_cursor_repopulate_debug_delay",
            delay_ms,
            "phase=l1_cursor_repopulate_debug_delay delay_ms={delay_ms}"
        );
        if ctx.run_until_cancelled(tokio::time::sleep(Duration::from_millis(delay_ms))).await.is_none() {
            return Ok(());
        }
    }

    let mut reconnect_delay = RECONNECT_BASE_DELAY;

    loop {
        let session_start = std::time::Instant::now();
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
            // This worker only observes L1 confirmations: an outage degrades finality
            // reporting but is never unsafe, so every error is retried rather than
            // aborting the node (which a returned Err would do via the ServiceMonitor).
            Some(Err(e)) => {
                if session_start.elapsed() >= RECONNECT_MAX_DELAY {
                    reconnect_delay = RECONNECT_BASE_DELAY;
                }
                if e.is_recoverable() {
                    tracing::warn!("L1 state sync failed: {e:#}, reconnecting in {reconnect_delay:?}");
                } else {
                    tracing::error!(
                        "L1 state sync failed with an unexpected error: {e:#}, reconnecting in {reconnect_delay:?}"
                    );
                }
                if ctx.run_until_cancelled(tokio::time::sleep(reconnect_delay)).await.is_none() {
                    return Ok(());
                }
                reconnect_delay = std::cmp::min(reconnect_delay * 2, RECONNECT_MAX_DELAY);
            }
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
    let fetch_started = Instant::now();
    tracing::info!(
        target: "startup_recovery",
        phase = "l1_state_fetch_started",
        "phase=l1_state_fetch_started"
    );
    let initial_state = settlement_client.get_current_core_contract_state().await?;
    let elapsed_ms = fetch_started.elapsed().as_secs_f64() * 1_000.0;
    tracing::info!(
        target: "startup_recovery",
        phase = "l1_state_fetch_completed",
        l1_confirmed_block = ?initial_state.block_number,
        elapsed_ms,
        "phase=l1_state_fetch_completed l1_confirmed_block={:?} elapsed_ms={elapsed_ms:.3}",
        initial_state.block_number
    );

    tracing::info!("Subscribed to L1 state verification");
    state.update_state(initial_state)?;

    settlement_client.listen_for_update_state_events(ctx, state).await
}

#[cfg(test)]
mod state_update_worker_tests {
    use super::*;
    use crate::client::MockSettlementLayerProvider;
    use mp_chain_config::ChainConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// End-to-end recovery test: when a state update subscription dies with the
    /// recoverable error produced by the liveness watchdog terminating a stuck
    /// stream (see `eth::event::LivenessStream`), `state_update_worker` must
    /// re-deploy a fresh subscription. This is the regression behind the
    /// 2026-06-05 incident, where the unguarded stream never terminated and L1
    /// state verification hung until a manual node restart.
    #[tokio::test(start_paused = true)]
    async fn worker_redeploys_subscription_after_liveness_termination() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let block_metrics = Arc::new(L1BlockMetrics::register().expect("metrics should register"));
        let ctx = ServiceContext::new_for_testing();
        let (l1_head_tx, l1_head_rx) = tokio::sync::watch::channel(None);

        let sessions = Arc::new(AtomicUsize::new(0));
        let mut mock = MockSettlementLayerProvider::new();
        mock.expect_get_current_core_contract_state().times(2).returning({
            let sessions = Arc::clone(&sessions);
            // Initial state per subscription: #100 for the first, #101 after reconnection.
            move || {
                let session = sessions.load(Ordering::SeqCst) as u64;
                Ok(StateUpdate { block_number: Some(100 + session), global_root: Felt::ZERO, block_hash: Felt::ZERO })
            }
        });
        mock.expect_listen_for_update_state_events().times(2).returning({
            let sessions = Arc::clone(&sessions);
            move |_ctx, worker| match sessions.fetch_add(1, Ordering::SeqCst) {
                // First subscription: delivers one event, then its poller gets stuck
                // and the liveness watchdog terminates the stream, surfacing the same
                // recoverable error as the real `listen_for_update_state_events`.
                0 => {
                    worker
                        .update_state(StateUpdate {
                            block_number: Some(200),
                            global_root: Felt::ZERO,
                            block_hash: Felt::ZERO,
                        })
                        .expect("update_state should succeed");
                    Err(SettlementClientError::StateEventListener(
                        "L1 state update event stream ended unexpectedly".to_string(),
                    ))
                }
                // Re-deployed subscription: healthy, ends cleanly.
                _ => Ok(()),
            }
        });

        // The worker must survive the first session's error, reconnect (the 1s
        // backoff is auto-advanced by the paused clock), and exit cleanly once the
        // second session completes. The `times(2)` expectations verify the
        // re-subscription; a worker that gives up after the error fails them.
        tokio::time::timeout(
            Duration::from_secs(30),
            state_update_worker(backend, Arc::new(mock), ctx, l1_head_tx, block_metrics),
        )
        .await
        .expect("worker should not hang after stream termination")
        .expect("worker should exit cleanly after the second session");

        // #100 (initial, session 1) → #200 (event) → reconnect → #101 (initial,
        // session 2): the last head proves the fresh subscription was used.
        assert_eq!(l1_head_rx.borrow().as_ref().and_then(|s| s.block_number), Some(101));
    }

    /// The worker must retry even on errors that are not classified recoverable:
    /// a transient RPC failure surfacing through an unclassified path previously
    /// propagated out of the worker and took down the whole node via the
    /// ServiceMonitor.
    #[tokio::test(start_paused = true)]
    async fn worker_retries_after_unclassified_error() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let block_metrics = Arc::new(L1BlockMetrics::register().expect("metrics should register"));
        let ctx = ServiceContext::new_for_testing();
        let (l1_head_tx, l1_head_rx) = tokio::sync::watch::channel(None);

        let sessions = Arc::new(AtomicUsize::new(0));
        let mut mock = MockSettlementLayerProvider::new();
        mock.expect_get_current_core_contract_state().times(2).returning({
            let sessions = Arc::clone(&sessions);
            move || match sessions.fetch_add(1, Ordering::SeqCst) {
                0 => Err(SettlementClientError::Other("transient failure on an unclassified path".to_string())),
                _ => Ok(StateUpdate { block_number: Some(100), global_root: Felt::ZERO, block_hash: Felt::ZERO }),
            }
        });
        mock.expect_listen_for_update_state_events().times(1).returning(|_ctx, _worker| Ok(()));

        tokio::time::timeout(
            Duration::from_secs(30),
            state_update_worker(backend, Arc::new(mock), ctx, l1_head_tx, block_metrics),
        )
        .await
        .expect("worker should not hang after an unclassified error")
        .expect("worker should exit cleanly after the second session");

        assert_eq!(l1_head_rx.borrow().as_ref().and_then(|s| s.block_number), Some(100));
    }

    #[test]
    fn transport_failures_in_contract_calls_are_recoverable() {
        use crate::eth::error::EthereumClientError;
        use crate::starknet::error::StarknetClientError;

        assert!(SettlementClientError::from(EthereumClientError::Contract(
            "Failed to get state root: transport error".to_string()
        ))
        .is_recoverable());
        assert!(SettlementClientError::from(StarknetClientError::StateInitialization {
            message: "Failed to get state: transport error".to_string()
        })
        .is_recoverable());
    }
}
