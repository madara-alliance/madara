use serde::{Deserialize, Serialize};

/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser, Deserialize, Serialize)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(env = "MADARA_BLOCK_PRODUCTION_DISABLED", long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,

    /// Enable the parallel merkle finalization pipeline.
    #[arg(env = "MADARA_PARALLEL_MERKLE_ENABLED", long, default_value_t = false)]
    pub parallel_merkle_enabled: bool,

    /// Flush interval for parallel merkle overlay persistence in number of blocks.
    #[arg(
        env = "MADARA_PARALLEL_MERKLE_FLUSH_INTERVAL",
        long,
        default_value_t = 3,
        value_parser = clap::value_parser!(u64).range(2..)
    )]
    pub parallel_merkle_flush_interval: u64,

    /// Maximum number of in-flight parallel merkle jobs.
    #[arg(
        env = "MADARA_PARALLEL_MERKLE_MAX_INFLIGHT",
        long,
        default_value_t = 10,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub parallel_merkle_max_inflight: u64,

    /// Trie-log mode for parallel merkle persistence.
    #[arg(env = "MADARA_PARALLEL_MERKLE_TRIE_LOG_MODE", long, default_value = "off")]
    pub parallel_merkle_trie_log_mode: ParallelMerkleTrieLogMode,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParallelMerkleTrieLogMode {
    Off,
    Checkpoint,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn block_production_params_defaults_parse() {
        let params = BlockProductionParams::try_parse_from(["madara"]).expect("defaults should parse");

        assert!(!params.parallel_merkle_enabled);
        assert_eq!(params.parallel_merkle_flush_interval, 3);
        assert_eq!(params.parallel_merkle_max_inflight, 10);
        assert_eq!(params.parallel_merkle_trie_log_mode, ParallelMerkleTrieLogMode::Off);
    }

    #[test]
    fn block_production_params_overrides_parse() {
        let params = BlockProductionParams::try_parse_from([
            "madara",
            "--parallel-merkle-enabled",
            "--parallel-merkle-flush-interval",
            "6",
            "--parallel-merkle-max-inflight",
            "42",
            "--parallel-merkle-trie-log-mode",
            "checkpoint",
        ])
        .expect("overrides should parse");

        assert!(params.parallel_merkle_enabled);
        assert_eq!(params.parallel_merkle_flush_interval, 6);
        assert_eq!(params.parallel_merkle_max_inflight, 42);
        assert_eq!(params.parallel_merkle_trie_log_mode, ParallelMerkleTrieLogMode::Checkpoint);
    }

    #[test]
    fn block_production_params_rejects_flush_interval_below_two() {
        let err = BlockProductionParams::try_parse_from(["madara", "--parallel-merkle-flush-interval", "1"])
            .expect_err("flush interval <2 must be rejected");

        let err_text = err.to_string();
        assert!(err_text.contains("2"));
        assert!(err_text.contains("parallel-merkle-flush-interval"));
    }
}
