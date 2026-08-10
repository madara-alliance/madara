/// T-053: Async re-execution dispatcher.
///
/// Runs as a Tokio task. Receives `ReexecRequest`s from block production, enforces a
/// bounded in-flight window (max 9 blocks, Starknet hash-window safety), and dispatches
/// each request as a `spawn_blocking` job to the blocking thread pool.
///
/// Cancellation:
/// - A `CancellationToken` per epoch is shared with all in-flight workers.
/// - Any fatal event (comparator stop, worker error, shutdown) cancels the token → all
///   workers for that epoch stop cooperatively.
/// - The caller obtains a new epoch by calling `new_epoch()`, which creates a fresh token.
use super::{worker, ReexecRequest, ReexecWorkerOutcome, MAX_INFLIGHT_REEXEC_BLOCKS};
use mp_utils::spawn_blocking;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

/// Handle to the re-execution dispatcher Tokio task.
///
/// Created by `start_reexec_dispatcher()`; used by block production to submit requests
/// and receive results.
#[derive(Debug)]
pub struct ReexecDispatcherHandle {
    /// Send re-execution requests into the dispatcher.
    pub req_tx: mpsc::Sender<ReexecRequest>,
    /// Receive re-execution results from the dispatcher.
    pub res_rx: mpsc::Receiver<anyhow::Result<ReexecWorkerOutcome>>,
    /// Cancel token for the current epoch. Cancel this to stop all in-flight workers.
    pub cancel_token: CancellationToken,
}

impl ReexecDispatcherHandle {
    /// Cancel all in-flight workers for the current epoch.
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

/// Start the re-execution dispatcher Tokio task.
///
/// Returns a `ReexecDispatcherHandle` that the block production task uses to send requests
/// and read results. The dispatcher task exits when `cancel_token` is cancelled or when
/// the `req_tx` is dropped.
pub fn start_reexec_dispatcher(cancel_token: CancellationToken) -> ReexecDispatcherHandle {
    let (req_tx, mut req_rx) = mpsc::channel::<ReexecRequest>(MAX_INFLIGHT_REEXEC_BLOCKS);
    let (res_tx, res_rx) = mpsc::channel::<anyhow::Result<ReexecWorkerOutcome>>(MAX_INFLIGHT_REEXEC_BLOCKS);

    let inflight_sem = Arc::new(Semaphore::new(MAX_INFLIGHT_REEXEC_BLOCKS));
    let cancel = cancel_token.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::debug!("reexec_dispatcher: epoch cancelled, stopping");
                    break;
                }
                req = req_rx.recv() => {
                    let Some(req) = req else {
                        tracing::debug!("reexec_dispatcher: request channel closed");
                        break;
                    };

                    // Acquire in-flight slot before spawning.
                    // This enforces backpressure: if MAX_INFLIGHT_REEXEC_BLOCKS are already
                    // running, this await blocks until one completes.
                    let permit = match inflight_sem.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => {
                            tracing::warn!("reexec_dispatcher: semaphore closed");
                            break;
                        }
                    };

                    let res_tx = res_tx.clone();
                    let cancel_worker = cancel.clone();

                    tokio::spawn(async move {
                        let _permit = permit; // released on drop when this task completes
                        let out = spawn_blocking(move || worker::run_blockifier_reexec(req, cancel_worker)).await;
                        // Flatten the outer anyhow::Result<anyhow::Result<ReexecWorkerOutcome>>.
                        // The outer error comes from JoinError (panic bubble-up), the inner from worker.
                        let result = out;
                        let _ = res_tx.send(result).await;
                    });
                }
            }
        }
    });

    ReexecDispatcherHandle { req_tx, res_rx, cancel_token }
}

/// Create a fresh epoch cancellation token (used when resuming after a stop).
pub fn new_epoch_token() -> CancellationToken {
    CancellationToken::new()
}
