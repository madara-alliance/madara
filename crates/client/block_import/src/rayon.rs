use std::{panic::AssertUnwindSafe, sync::atomic::AtomicUsize, thread};
use tokio::sync::Semaphore;

/// Wraps the rayon pool in a tokio-friendly way.
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
        tracing::debug!("acquire permit {permit_id}");
        let permit = self.semaphore.acquire().await.expect("Poisoned semaphore");
        let n_acquired_permits = self.n_acquired_permits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        tracing::debug!("acquired permit {permit_id} ({n_acquired_permits}/{max_tasks})");

        let res = global_spawn_rayon_task(func).await;

        drop(permit);

        let n_acquired_permits = self.n_acquired_permits.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        tracing::debug!("released permit {permit_id} ({n_acquired_permits}/{max_tasks})");
        res
    }
}

pub async fn global_spawn_rayon_task<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Important: fifo mode.
    rayon::spawn_fifo(move || {
        // We bubble up the panics to the tokio pool.
        let _result = tx.send(std::panic::catch_unwind(AssertUnwindSafe(func)));
    });

    rx.await.expect("Tokio channel closed").expect("Rayon task panicked")
}
