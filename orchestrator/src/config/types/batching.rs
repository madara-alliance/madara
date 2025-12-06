use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchingConfig {
    pub max_batch_time_seconds: u64,
    pub max_batch_size: u64,

    #[serde(default = "default_max_blobs")]
    pub max_num_blobs: usize,

    #[serde(default = "default_lock_duration")]
    pub lock_duration_seconds: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_blocks_per_snos_batch: Option<u64>,

    #[serde(default = "default_max_snos_batches")]
    pub max_snos_batches_per_aggregator: u64,

    #[serde(default = "default_empty_block_gas")]
    pub default_empty_block_proving_gas: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bouncer_weights_file: Option<std::path::PathBuf>,
}

fn default_max_blobs() -> usize {
    6
}
fn default_lock_duration() -> u64 {
    3600
}
fn default_max_snos_batches() -> u64 {
    50
}
fn default_empty_block_gas() -> u64 {
    1500000
}
