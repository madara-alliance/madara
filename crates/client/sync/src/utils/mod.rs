#![macro_use]
#![allow(clippy::new_without_default)]
use std::time::{Duration, Instant};

pub mod constant;
pub mod convert;
#[cfg(feature = "m")]
pub mod m;
pub mod utility;

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
