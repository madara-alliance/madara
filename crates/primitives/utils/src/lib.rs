#![allow(clippy::new_without_default)]

pub mod service;
pub mod tests_common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::Future;
use tokio::sync::oneshot;

/// Prefer this compared to [`tokio::spawn_blocking`], as spawn_blocking creates new OS threads and
/// we don't really need that
pub async fn spawn_rayon_task<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();

    rayon::spawn(move || {
        let _result = tx.send(func());
    });

    rx.await.expect("tokio channel closed")
}

static CTRL_C: AtomicBool = AtomicBool::new(false);

async fn graceful_shutdown_inner() {
    let sigint = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => signal.recv().await,
            // SIGTERM not supported
            Err(_) => core::future::pending().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigint => {},
    };
    CTRL_C.store(true, Ordering::SeqCst);
}
pub async fn graceful_shutdown() {
    if CTRL_C.load(Ordering::SeqCst) {
        return;
    }
    graceful_shutdown_inner().await
}

/// Should be used with streams/channels `next`/`recv` function.
pub async fn wait_or_graceful_shutdown<T>(future: impl Future<Output = T>) -> Option<T> {
    if CTRL_C.load(Ordering::SeqCst) {
        return None;
    }
    tokio::select! {
        _ = graceful_shutdown_inner() => { None },
        res = future => { Some(res) },
    }
}

/// Should be used with streams/channels `next`/`recv` function.
pub async fn channel_wait_or_graceful_shutdown<T>(future: impl Future<Output = Option<T>>) -> Option<T> {
    wait_or_graceful_shutdown(future).await?
}

#[derive(Debug, Default)]
pub struct StopHandle(Option<oneshot::Sender<()>>);

impl StopHandle {
    pub fn new(inner: Option<oneshot::Sender<()>>) -> Self {
        Self(inner)
    }
}
impl Drop for StopHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _res = sender.send(());
        }
    }
}

pub struct PerfStopwatch(pub Instant);

impl PerfStopwatch {
    pub fn new() -> PerfStopwatch {
        PerfStopwatch(Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

#[macro_export]
macro_rules! stopwatch_end {
    ($stopwatch:expr, $($arg:tt)+) => {
        log::debug!($($arg)+, $stopwatch.elapsed())
    }
}
