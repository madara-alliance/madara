#![macro_use]
#![allow(clippy::new_without_default)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::Future;

pub mod constant;
pub mod convert;
#[cfg(feature = "m")]
pub mod m;
pub mod utility;

static CTRL_C: AtomicBool = AtomicBool::new(false);

/// Should be used with streams/channels `next`/`recv` function.
pub async fn wait_or_graceful_shutdown<T>(future: impl Future<Output = T>) -> Option<T> {
    if CTRL_C.load(Ordering::SeqCst) {
        return None;
    }
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            CTRL_C.store(true, Ordering::SeqCst);
            None
        }
        res = future => {
            Some(res)
        },
    }
}

/// Should be used with streams/channels `next`/`recv` function.
pub async fn channel_wait_or_graceful_shutdown<T>(future: impl Future<Output = Option<T>>) -> Option<T> {
    wait_or_graceful_shutdown(future).await?
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
