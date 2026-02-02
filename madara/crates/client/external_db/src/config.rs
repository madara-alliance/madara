//! Configuration structures for external database integration.

use serde::{Deserialize, Serialize};

fn default_database_name() -> String {
    "madara".to_string()
}

fn default_collection_name() -> String {
    "mempool_transactions".to_string()
}

fn default_batch_size() -> usize {
    100
}

fn default_flush_interval_ms() -> u64 {
    1000
}

fn default_pool_size() -> u32 {
    10
}

fn default_min_pool_size() -> u32 {
    1
}

fn default_connect_timeout_secs() -> u64 {
    10
}

fn default_server_selection_timeout_secs() -> u64 {
    10
}

fn default_retention_delay_secs() -> u64 {
    86_400 // 1 day
}

fn default_retention_tick_secs() -> u64 {
    300 // 5 minutes
}

fn default_retry_backoff_ms() -> u64 {
    1000
}

fn default_retry_backoff_max_ms() -> u64 {
    60_000
}

fn default_strict_outbox() -> bool {
    true
}

/// Configuration for external database integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalDbConfig {
    /// Enable external database storage
    pub enabled: bool,

    /// MongoDB connection URI
    /// Example: "mongodb://localhost:27017"
    pub mongodb_uri: String,

    /// Database name
    /// Default: "madara_<chain_id>" (chain_id substituted at runtime)
    #[serde(default = "default_database_name")]
    pub database_name: String,

    /// Collection name for mempool transactions
    #[serde(default = "default_collection_name")]
    pub collection_name: String,

    /// Batch size for bulk writes
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Flush interval in milliseconds
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Minimum connection pool size
    #[serde(default = "default_min_pool_size")]
    pub min_pool_size: u32,

    /// MongoDB connect timeout in seconds
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,

    /// MongoDB server selection timeout in seconds
    #[serde(default = "default_server_selection_timeout_secs")]
    pub server_selection_timeout_secs: u64,

    /// Retention: delete txs N seconds after their block is confirmed on L1
    #[serde(default = "default_retention_delay_secs")]
    pub retention_delay_secs: u64,

    /// How often the retention sweeper checks for deletions
    #[serde(default = "default_retention_tick_secs")]
    pub retention_tick_secs: u64,

    /// Required: failure to persist to outbox rejects mempool acceptance
    #[serde(default = "default_strict_outbox")]
    pub strict_outbox: bool,

    /// Retry backoff base (milliseconds)
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,

    /// Retry backoff max (milliseconds)
    #[serde(default = "default_retry_backoff_max_ms")]
    pub retry_backoff_max_ms: u64,
}

impl Default for ExternalDbConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mongodb_uri: String::new(),
            database_name: default_database_name(),
            collection_name: default_collection_name(),
            batch_size: default_batch_size(),
            flush_interval_ms: default_flush_interval_ms(),
            pool_size: default_pool_size(),
            min_pool_size: default_min_pool_size(),
            connect_timeout_secs: default_connect_timeout_secs(),
            server_selection_timeout_secs: default_server_selection_timeout_secs(),
            retention_delay_secs: default_retention_delay_secs(),
            retention_tick_secs: default_retention_tick_secs(),
            strict_outbox: default_strict_outbox(),
            retry_backoff_ms: default_retry_backoff_ms(),
            retry_backoff_max_ms: default_retry_backoff_max_ms(),
        }
    }
}

impl ExternalDbConfig {
    /// Creates a new config with the given MongoDB URI.
    pub fn new(mongodb_uri: String) -> Self {
        Self { enabled: true, mongodb_uri, ..Default::default() }
    }

    /// Sets the database name, substituting `<chain_id>` placeholder if present.
    pub fn with_chain_id(mut self, chain_id: &str) -> Self {
        if self.database_name.contains("<chain_id>") {
            self.database_name = self.database_name.replace("<chain_id>", chain_id);
        } else if self.database_name == "madara" {
            // Default database name: append chain_id
            self.database_name = format!("madara_{}", chain_id);
        }
        self
    }

    /// Returns true if the config is valid for starting the service.
    pub fn is_valid(&self) -> bool {
        self.enabled && !self.mongodb_uri.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ExternalDbConfig::default();
        assert!(!config.enabled);
        assert!(config.mongodb_uri.is_empty());
        assert_eq!(config.database_name, "madara");
        assert_eq!(config.collection_name, "mempool_transactions");
        assert_eq!(config.batch_size, 100);
        assert_eq!(config.flush_interval_ms, 1000);
        assert_eq!(config.pool_size, 10);
        assert_eq!(config.min_pool_size, 1);
        assert_eq!(config.connect_timeout_secs, 10);
        assert_eq!(config.server_selection_timeout_secs, 10);
        assert_eq!(config.retention_delay_secs, 86_400);
        assert_eq!(config.retention_tick_secs, 300);
        assert!(config.strict_outbox);
        assert_eq!(config.retry_backoff_ms, 1000);
        assert_eq!(config.retry_backoff_max_ms, 60_000);
    }

    #[test]
    fn test_with_chain_id() {
        let config = ExternalDbConfig::default().with_chain_id("mainnet");
        assert_eq!(config.database_name, "madara_mainnet");

        let config = ExternalDbConfig { database_name: "my_<chain_id>_db".to_string(), ..Default::default() }
            .with_chain_id("testnet");
        assert_eq!(config.database_name, "my_testnet_db");
    }

    #[test]
    fn test_is_valid() {
        let config = ExternalDbConfig::default();
        assert!(!config.is_valid());

        let config = ExternalDbConfig::new("mongodb://localhost:27017".to_string());
        assert!(config.is_valid());

        let config = ExternalDbConfig { enabled: true, mongodb_uri: String::new(), ..Default::default() };
        assert!(!config.is_valid());
    }
}
