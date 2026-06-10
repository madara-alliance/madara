use std::{
    panic::AssertUnwindSafe,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};
use tokio::sync::{Notify, Semaphore};

/// Wraps the rayon pool in a tokio-friendly way.
///
/// This should be avoided in RPC/p2p/any other end-user endpoints, as this could be a DoS vector. To avoid that,
/// signature verification should probably be done before sending to the rayon pool
/// As a safety, a semaphore is added to bound the queue and support backpressure.
/// The tasks are added in FIFO order.
pub struct RayonPool {
    semaphore: Semaphore,
    max_tasks: usize,
    permit_id: AtomicUsize,
    n_acquired_permits: AtomicUsize,
}

impl Default for RayonPool {
    fn default() -> Self {
        Self::new()
    }
}

impl RayonPool {
    pub fn new() -> Self {
        let n_cores = thread::available_parallelism().expect("Getting the number of cores").get();
        let max_tasks = n_cores * 2;
        Self { semaphore: Semaphore::new(max_tasks), max_tasks, permit_id: 0.into(), n_acquired_permits: 0.into() }
    }

    pub async fn spawn_rayon_task<F, R>(&self, func: F) -> R
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let max_tasks = self.max_tasks;
        let permit_id = self.permit_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tracing::trace!("acquire permit {permit_id}");
        let permit = self.semaphore.acquire().await.expect("Poisoned semaphore");
        let n_acquired_permits = self.n_acquired_permits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        tracing::trace!("acquired permit {permit_id} ({n_acquired_permits}/{max_tasks})");

        let res = global_spawn_rayon_task(func).await;

        drop(permit);

        let n_acquired_permits = self.n_acquired_permits.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        tracing::trace!("released permit {permit_id} ({n_acquired_permits}/{max_tasks})");
        res
    }
}

/// Tracks tasks that are currently queued or running on the global rayon pool through
/// [`global_spawn_rayon_task`].
///
/// Rayon worker threads are detached: dropping the future returned by
/// [`global_spawn_rayon_task`] (e.g. on graceful-shutdown cancellation) does *not* stop the
/// task once it has been handed to the pool. Anything that wants to tear down resources those
/// tasks may be using (the database backend, the process itself) must first wait for the pool
/// to drain using [`wait_global_rayon_tasks_finished`]. See issue #1091: exiting the process
/// while a snap-sync trie-apply task was still running inside RocksDB caused a SIGSEGV during
/// process teardown.
struct GlobalRayonTaskTracker {
    in_flight: AtomicUsize,
    notify: Notify,
}

static GLOBAL_RAYON_TASKS: GlobalRayonTaskTracker =
    GlobalRayonTaskTracker { in_flight: AtomicUsize::new(0), notify: Notify::const_new() };

/// Decrements the in-flight count when the rayon task finishes, even if it panics.
struct InFlightGuard;

impl InFlightGuard {
    fn register() -> Self {
        GLOBAL_RAYON_TASKS.in_flight.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // Only the last task in flight wakes drain waiters; earlier completions leave the
        // counter nonzero, so `wait_global_rayon_tasks_finished` must keep waiting anyway.
        if GLOBAL_RAYON_TASKS.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            GLOBAL_RAYON_TASKS.notify.notify_waiters();
        }
    }
}

/// Number of tasks currently queued or running on the global rayon pool via
/// [`global_spawn_rayon_task`]. Note that this includes tasks whose awaiting future has been
/// dropped (cancelled): the underlying rayon work keeps running regardless.
pub fn global_rayon_tasks_in_flight() -> usize {
    GLOBAL_RAYON_TASKS.in_flight.load(Ordering::SeqCst)
}

/// Waits until no task spawned through [`global_spawn_rayon_task`] is queued or running
/// anymore.
///
/// This must be awaited before dropping resources shared with rayon tasks or exiting the
/// process, as cancelling the futures returned by [`global_spawn_rayon_task`] does not cancel
/// the detached rayon work itself.
pub async fn wait_global_rayon_tasks_finished() {
    loop {
        // Register the waiter *before* checking the counter, so a task finishing in between
        // cannot be missed (`notify_waiters` only wakes already-registered waiters).
        let notified = GLOBAL_RAYON_TASKS.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        if GLOBAL_RAYON_TASKS.in_flight.load(Ordering::SeqCst) == 0 {
            return;
        }

        notified.await;
    }
}

pub async fn global_spawn_rayon_task<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Register synchronously, before the task is handed to rayon, so that the task is always
    // visible to `wait_global_rayon_tasks_finished` — even if the future we return is dropped
    // (cancelled) before the rayon task gets a chance to run.
    let guard = InFlightGuard::register();

    // Important: fifo mode.
    rayon::spawn_fifo(move || {
        // We bubble up the panics to the tokio pool.
        let _result = tx.send(std::panic::catch_unwind(AssertUnwindSafe(func)));
        drop(guard);
    });

    match rx.await.expect("Tokio channel closed") {
        Ok(r) => r,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Regression test for issue #1091: a task offloaded to the global rayon pool keeps
    /// running even when the future awaiting it is dropped (as happens when a service is
    /// cancelled during graceful shutdown). The shutdown path must be able to observe that
    /// work and wait for it to finish before tearing down shared resources (e.g. the database
    /// backend) or exiting the process.
    #[tokio::test]
    async fn dropped_future_is_still_tracked_until_rayon_task_finishes() {
        // Note: the tracker is global and tests may run concurrently within this crate, so we
        // assert on deltas/joins rather than absolute zero where possible.
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();

        let mut task = Box::pin(global_spawn_rayon_task(move || {
            started_tx.send(()).expect("send started");
            // Block the rayon worker until the test releases it, simulating a long
            // snap-sync trie-apply batch.
            release_rx.recv_timeout(Duration::from_secs(30)).expect("release signal");
        }));

        // Poll once so the task is handed to the rayon pool (this is what awaiting does), then
        // simulate graceful-shutdown cancellation: the awaiting future is dropped while the
        // rayon task is queued or running.
        assert!(futures::poll!(task.as_mut()).is_pending());
        drop(task);

        // Wait until the task is actually running on the pool.
        started_rx.recv_timeout(Duration::from_secs(30)).expect("task should start");
        assert!(global_rayon_tasks_in_flight() >= 1, "cancelled task must still be tracked");

        // While the task is blocked, the drain must not complete.
        let drain = tokio::time::timeout(Duration::from_millis(100), wait_global_rayon_tasks_finished()).await;
        assert!(drain.is_err(), "drain must wait for the in-flight rayon task");

        // Release the task: the drain must now complete.
        release_tx.send(()).expect("send release");
        tokio::time::timeout(Duration::from_secs(30), wait_global_rayon_tasks_finished())
            .await
            .expect("drain should complete once the rayon task finishes");
    }

    #[tokio::test]
    async fn panicking_task_is_untracked() {
        let task = global_spawn_rayon_task(|| panic!("boom"));
        // The panic is bubbled up; catch it so the test can continue.
        assert!(futures::FutureExt::catch_unwind(AssertUnwindSafe(task)).await.is_err());
        tokio::time::timeout(Duration::from_secs(30), wait_global_rayon_tasks_finished())
            .await
            .expect("panicked task must still decrement the in-flight count");
    }
}
