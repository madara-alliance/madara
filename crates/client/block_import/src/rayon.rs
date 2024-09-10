use std::{panic::AssertUnwindSafe, thread};
use tokio::sync::Semaphore;

/// Wraps the rayon pool in a tokio-friendly way.
/// This should be avoided in RPC/p2p/any other end-user endpoints, as this could be a DoS vector. To avoid that,
/// signature verification should probably be done before sending to the rayon pool
/// As a safety, a semaphore is added to bound the queue and support backpressure.
/// The tasks are added in FIFO order.
pub struct RayonPool {
    semaphore: Semaphore,
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
        Self { semaphore: Semaphore::new(max_tasks) }
    }

    pub async fn spawn_rayon_task<F, R>(&self, func: F) -> R
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let _permit = self.semaphore.acquire().await;

        let (tx, rx) = tokio::sync::oneshot::channel();

        // Important: fifo mode.
        rayon::spawn_fifo(move || {
            // We bubble up the panics to the tokio pool.
            let _result = tx.send(std::panic::catch_unwind(AssertUnwindSafe(func)));
        });

        rx.await.expect("tokio channel closed").expect("rayon task panicked")
    }
}
