#![allow(clippy::new_without_default)]

pub mod crypto;
pub mod hash;
pub mod parsers;
pub mod serde;
pub mod service;
use std::time::{Duration, Instant};

pub use hash::trim_hash;

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
