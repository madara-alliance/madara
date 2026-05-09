use std::time::Duration;

pub const CAIRO_PIE_PATH: &str = "/tests/artifacts/fibonacci.zip";

// Live Atlantic jobs can queue for several minutes before completing.
pub const MAX_RETRIES: u8 = 90;
pub const RETRY_DELAY: Duration = std::time::Duration::from_secs(10);
