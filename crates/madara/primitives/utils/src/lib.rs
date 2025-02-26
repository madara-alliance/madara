#![allow(clippy::new_without_default)]

pub mod crypto;
pub mod hash;
pub mod parsers;
pub mod rayon;
pub mod serde;
pub mod service;
use std::time::{Duration, Instant};

pub use hash::trim_hash;

use tokio::sync::oneshot;

#[repr(transparent)]
struct Frozen<T>(T);

impl<T> std::ops::Deref for Frozen<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Default> Default for Frozen<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> Frozen<T> {
    fn new(obj: T) -> Self {
        Self(obj)
    }

    fn into_inner(self) -> T {
        self.0
    }
}

trait Freeze<T>
where
    Self: Sized,
{
    fn freeze(self) -> Frozen<T>;
}

impl<T: Sized> Freeze<T> for T {
    fn freeze(self) -> Frozen<T> {
        Frozen(self)
    }
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
