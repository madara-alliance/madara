use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, clap::ValueEnum, Deserialize, Serialize, PartialEq, Eq)]
pub enum ParallelMerkleTrieLogMode {
    Off,
    Checkpoint,
}

/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser, Deserialize, Serialize)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(env = "MADARA_BLOCK_PRODUCTION_DISABLED", long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Start with mempool intake paused.
    ///
    /// Requires `--rpc-admin --rpc-unsafe`.
    #[arg(env = "MADARA_MEMPOOL_PAUSED", long)]
    pub mempool_paused: bool,

    /// Enable replay mode for block production.
    ///
    /// In replay mode, configured replay boundaries are used to constrain batching/execution so
    /// transaction ingestion does not cross source block boundaries.
    #[arg(env = "MADARA_REPLAY_MODE", long, default_value_t = false)]
    pub replay_mode: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,

    /// Maximum number of in-flight close-block jobs accepted by the close queue.
    /// This controls queue backpressure independently from tx execution concurrency.
    #[arg(
        env = "MADARA_PARALLEL_MERKLE_MAX_INFLIGHT",
        long,
        default_value_t = 10,
        value_parser = clap::value_parser!(u64).range(10..)
    )]
    pub parallel_merkle_max_inflight: u64,

    /// Enable parallel merkle root computation.
    /// When enabled, trie root computation runs in a dedicated worker thread
    /// while commitments, header, hash, and DB writes remain inline.
    #[arg(env = "MADARA_PARALLEL_MERKLE_ENABLED", long, default_value_t = false)]
    pub parallel_merkle_enabled: bool,

    /// Parallel merkle boundary flush interval (in blocks).
    #[arg(
        env = "MADARA_PARALLEL_MERKLE_FLUSH_INTERVAL",
        long,
        default_value_t = 3,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub parallel_merkle_flush_interval: u64,

    /// Parallel merkle trie-log persistence mode.
    #[arg(
        env = "MADARA_PARALLEL_MERKLE_TRIE_LOG_MODE",
        long,
        default_value_t = ParallelMerkleTrieLogMode::Checkpoint,
        value_enum
    )]
    pub parallel_merkle_trie_log_mode: ParallelMerkleTrieLogMode,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rstest::rstest;

    #[rstest]
    #[case::defaults(vec!["madara"], 10)]
    #[case::override_value(vec!["madara", "--parallel-merkle-max-inflight", "42"], 42)]
    fn block_production_params_parse_parallel_merkle_max_inflight(
        #[case] args: Vec<&str>,
        #[case] expected_max_inflight: u64,
    ) {
        let params = BlockProductionParams::try_parse_from(args).expect("arguments should parse");
        assert_eq!(params.parallel_merkle_max_inflight, expected_max_inflight);
    }

    #[rstest]
    #[case::below_minimum_zero("0")]
    #[case::below_minimum_nine("9")]
    fn block_production_params_rejects_invalid_max_inflight(#[case] invalid_value: &str) {
        let err = BlockProductionParams::try_parse_from(["madara", "--parallel-merkle-max-inflight", invalid_value])
            .expect_err("max inflight <10 must be rejected");

        let err_text = err.to_string();
        assert!(err_text.contains("parallel-merkle-max-inflight"));
    }

    #[rstest]
    #[case::zero("0")]
    fn block_production_params_rejects_invalid_flush_interval(#[case] invalid_value: &str) {
        let err = BlockProductionParams::try_parse_from(["madara", "--parallel-merkle-flush-interval", invalid_value])
            .expect_err("flush interval <1 must be rejected");
        assert!(err.to_string().contains("parallel-merkle-flush-interval"));
    }

    #[rstest]
    #[case::default_mode(vec!["madara"], ParallelMerkleTrieLogMode::Checkpoint)]
    #[case::off_mode(vec!["madara", "--parallel-merkle-trie-log-mode", "off"], ParallelMerkleTrieLogMode::Off)]
    fn block_production_params_parse_parallel_merkle_trie_log_mode(
        #[case] args: Vec<&str>,
        #[case] expected: ParallelMerkleTrieLogMode,
    ) {
        let params = BlockProductionParams::try_parse_from(args).expect("arguments should parse");
        assert_eq!(params.parallel_merkle_trie_log_mode, expected);
    }

    #[rstest]
    #[case::default_disabled(vec!["madara"], false)]
    #[case::enabled(vec!["madara", "--replay-mode"], true)]
    fn block_production_params_parse_replay_mode(#[case] args: Vec<&str>, #[case] expected: bool) {
        let params = BlockProductionParams::try_parse_from(args).expect("arguments should parse");
        assert_eq!(params.replay_mode, expected);
    }
}
