use std::time::Duration;

pub const CAIRO_PIE_PATH: &str = "/tests/artifacts/fibonacci.zip";

// Poll for job status until it's done or timeout is reached
pub const MAX_RETRIES: u8 = 30; // Set a reasonable number of retries
pub const RETRY_DELAY: Duration = std::time::Duration::from_secs(10);
