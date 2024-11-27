#![allow(clippy::new_without_default)]

pub mod crypto;
pub mod parsers;
pub mod serde;
pub mod service;

use std::time::{Duration, Instant};

use futures::Future;
use service::ServiceContext;
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

async fn graceful_shutdown_inner(ctx: &ServiceContext) {
    let sigterm = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => signal.recv().await,
            // SIGTERM not supported
            Err(_) => core::future::pending().await,
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm => {},
        _ = ctx.cancelled() => {},
    };

    ctx.cancel_local()
}
pub async fn graceful_shutdown(ctx: &ServiceContext) {
    graceful_shutdown_inner(ctx).await
}

/// Should be used with streams/channels `next`/`recv` function.
pub async fn wait_or_graceful_shutdown<T>(future: impl Future<Output = T>, ctx: &ServiceContext) -> Option<T> {
    tokio::select! {
        _ = graceful_shutdown_inner(ctx) => { None },
        res = future => { Some(res) },
    }
}

/// Should be used with streams/channels `next`/`recv` function.
pub async fn channel_wait_or_graceful_shutdown<T>(
    future: impl Future<Output = Option<T>>,
    ctx: &ServiceContext,
) -> Option<T> {
    wait_or_graceful_shutdown(future, ctx).await?
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
    #[tracing::instrument(name = "PerfStopwatch::new")]
    pub fn new() -> PerfStopwatch {
        PerfStopwatch(Instant::now())
    }

    #[tracing::instrument(name = "PerfStopwatch::elapsed", skip(self))]
    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

#[macro_export]
macro_rules! stopwatch_end {
    ($stopwatch:expr, $($arg:tt)+) => {
        tracing::debug!($($arg)+, $stopwatch.elapsed())
    }
}
