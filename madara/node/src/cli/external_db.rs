use clap::Args;
use serde::{Deserialize, Serialize};

/// Parameters used to configure external database storage for mempool transactions.
#[derive(Debug, Clone, Args, Deserialize, Serialize)]
pub struct ExternalDbParams {
    /// Enable external database storage for mempool transactions.
    #[arg(env = "MADARA_EXTERNAL_DB_ENABLED", long, default_value = "false")]
    pub external_db_enabled: bool,

    /// MongoDB connection URI.
    /// Example: "mongodb://localhost:27017"
    #[arg(env = "MADARA_EXTERNAL_DB_MONGODB_URI", long)]
    pub external_db_mongodb_uri: Option<String>,

    /// Database name.
    /// Default: "madara_<chain_id>"
    #[arg(env = "MADARA_EXTERNAL_DB_DATABASE", long)]
    pub external_db_database: Option<String>,

    /// Collection name for mempool transactions.
    #[arg(env = "MADARA_EXTERNAL_DB_COLLECTION", long, default_value = "mempool_transactions")]
    pub external_db_collection: String,

    /// Batch size for bulk writes.
    #[arg(env = "MADARA_EXTERNAL_DB_BATCH_SIZE", long, default_value = "100")]
    pub external_db_batch_size: usize,

    /// Flush interval in milliseconds.
    #[arg(env = "MADARA_EXTERNAL_DB_FLUSH_INTERVAL_MS", long, default_value = "1000")]
    pub external_db_flush_interval_ms: u64,

    /// Retention delay after L1 confirmation (seconds).
    /// Transactions are deleted this many seconds after their block is confirmed on L1.
    #[arg(env = "MADARA_EXTERNAL_DB_RETENTION_SECS", long, default_value = "86400")]
    pub external_db_retention_secs: u64,

    /// Retention sweeper interval (seconds).
    /// How often the retention sweeper checks for deletions.
    #[arg(env = "MADARA_EXTERNAL_DB_RETENTION_TICK_SECS", long, default_value = "300")]
    pub external_db_retention_tick_secs: u64,

    /// Reject mempool acceptance if outbox write fails.
    /// When enabled, transactions are only accepted into mempool if they can be
    /// durably persisted to the local outbox first.
    #[arg(env = "MADARA_EXTERNAL_DB_STRICT_OUTBOX", long, default_value = "true")]
    pub external_db_strict_outbox: bool,

    /// Base retry backoff in milliseconds.
    /// Used for exponential backoff when MongoDB writes fail.
    #[arg(env = "MADARA_EXTERNAL_DB_RETRY_BACKOFF_MS", long, default_value = "1000")]
    pub external_db_retry_backoff_ms: u64,

    /// Maximum retry backoff in milliseconds.
    /// Caps the exponential backoff to prevent excessively long waits.
    #[arg(env = "MADARA_EXTERNAL_DB_RETRY_BACKOFF_MAX_MS", long, default_value = "60000")]
    pub external_db_retry_backoff_max_ms: u64,
}

impl Default for ExternalDbParams {
    fn default() -> Self {
        Self {
            external_db_enabled: false,
            external_db_mongodb_uri: None,
            external_db_database: None,
            external_db_collection: "mempool_transactions".to_string(),
            external_db_batch_size: 100,
            external_db_flush_interval_ms: 1000,
            external_db_retention_secs: 86_400,
            external_db_retention_tick_secs: 300,
            external_db_strict_outbox: true,
            external_db_retry_backoff_ms: 1000,
            external_db_retry_backoff_max_ms: 60_000,
        }
    }
}

impl ExternalDbParams {
    /// Returns true if external DB is enabled and configured.
    pub fn is_enabled(&self) -> bool {
        self.external_db_enabled && self.external_db_mongodb_uri.is_some()
    }
}
