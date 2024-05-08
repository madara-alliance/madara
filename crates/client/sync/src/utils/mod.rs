#![macro_use]
#![allow(clippy::new_without_default)]
use std::time::Instant;

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
}

#[macro_export]
macro_rules! stopwatch_end {
    ($stopwatch:expr, $($arg:tt)+) => {
        log::debug!($($arg)+, $stopwatch.0.elapsed())
    }
}
