#[allow(dead_code)]
pub const ETHEREUM_MAX_BYTES_PER_BLOB: u64 = 131072;
#[allow(dead_code)]
pub const ETHEREUM_MAX_BLOB_PER_TXN: u64 = 6;

/// Maximum number of retries for queue message consumption in tests
pub const QUEUE_CONSUME_MAX_RETRIES: usize = 10;

/// Delay in seconds between retries for queue message consumption in tests
pub const QUEUE_CONSUME_RETRY_DELAY_SECS: u64 = 2;

/// Maximum number of retries for queue message consumption in tests (alternative configuration)
pub const QUEUE_CONSUME_MAX_RETRIES_ALT: usize = 5;

/// Delay in seconds between retries for queue message consumption in tests (alternative configuration)
pub const QUEUE_CONSUME_RETRY_DELAY_SECS_ALT: u64 = 1;
