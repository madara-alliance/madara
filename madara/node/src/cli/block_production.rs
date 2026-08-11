use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

fn normalize_hex_felt(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("felt value cannot be empty".to_string());
    }

    let normalized = if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        trimmed.to_string()
    } else {
        format!("0x{trimmed}")
    };

    Felt::from_hex(&normalized).map_err(|err| format!("invalid felt hex '{raw}': {err:?}"))?;
    Ok(normalized)
}

pub(crate) fn parse_hex_felt(raw: &str) -> Result<Felt, String> {
    let normalized = normalize_hex_felt(raw)?;
    Felt::from_hex(&normalized).map_err(|err| format!("invalid felt hex '{raw}': {err:?}"))
}

fn parse_hex_felt_arg(raw: &str) -> Result<String, String> {
    normalize_hex_felt(raw)
}

fn default_rust_exec_executor_addresses() -> Vec<String> {
    vec!["0x012aa6059457fc2d02240962a6573e051fa919632853e6ba70207d7cef6be4c3".to_string()]
}

fn default_rust_exec_batch_size() -> u64 {
    30
}

fn default_rust_exec_blockifier_batch_size() -> u64 {
    10
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, clap::Parser, Deserialize, Serialize)]
pub struct RustExecParams {
    /// Sender-address whitelist used for Rust execution routing.
    ///
    /// Values are comma-delimited felts.
    #[arg(
        env = "MADARA_RUST_EXEC_EXECUTOR_ADDRESSES",
        long,
        value_delimiter = ',',
        value_parser = parse_hex_felt_arg,
        default_values_t = default_rust_exec_executor_addresses(),
        value_name = "ADDRESS[,ADDRESS...]"
    )]
    #[serde(default = "default_rust_exec_executor_addresses")]
    pub executor_addresses: Vec<String>,

    /// Maximum Rust-routed transactions picked in one batcher cycle.
    #[arg(
        env = "MADARA_RUST_EXEC_BATCH_SIZE",
        long,
        default_value_t = default_rust_exec_batch_size(),
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    #[serde(default = "default_rust_exec_batch_size")]
    pub batch_size: u64,

    /// Maximum Blockifier-routed transactions picked in one batcher cycle.
    #[arg(
        env = "MADARA_RUST_EXEC_BLOCKIFIER_BATCH_SIZE",
        long,
        default_value_t = default_rust_exec_blockifier_batch_size(),
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    #[serde(default = "default_rust_exec_blockifier_batch_size")]
    pub blockifier_batch_size: u64,

    /// Enable per-transaction Rust execution timing logs.
    #[arg(env = "MADARA_RUST_EXEC_EXECUTION_LOG", long, default_value_t = false)]
    #[serde(default)]
    pub execution_log: bool,

    /// Enable inner Paraclear execution logs.
    #[arg(env = "MADARA_RUST_EXEC_EXECUTION_LOG_INNER", long, default_value_t = false)]
    #[serde(default)]
    pub execution_log_inner: bool,

    /// Enable Rust transaction state-diff summary logs.
    #[arg(env = "MADARA_RUST_EXEC_TX_DIFF_LOG", long, default_value_t = false)]
    #[serde(default)]
    pub tx_diff_log: bool,

    /// Enable Rust transaction state-diff summary logs for one block.
    #[arg(env = "MADARA_RUST_EXEC_DEBUG_BLOCK", long)]
    #[serde(default)]
    pub debug_block: Option<u64>,

    /// Enable detailed inner Paraclear timing logs.
    #[arg(env = "MADARA_RUST_EXEC_INNER_TIMING_LOG", long, default_value_t = false)]
    #[serde(default)]
    pub inner_timing_log: bool,

    /// Enable the per-transaction storage read cache.
    #[arg(env = "MADARA_RUST_EXEC_CTX_CACHE", long, default_value_t = true)]
    #[serde(default = "default_true")]
    pub ctx_cache: bool,

    /// Enable Pedersen/key derivation caching.
    #[arg(env = "MADARA_RUST_EXEC_PEDERSEN_CACHE", long, default_value_t = true)]
    #[serde(default = "default_true")]
    pub pedersen_cache: bool,

    /// Enable precomputed sn_keccak lookup for known Paradex names.
    #[arg(env = "MADARA_RUST_EXEC_PRECOMPUTED_SN_KECCAK", long, default_value_t = false)]
    #[serde(default)]
    pub precomputed_sn_keccak: bool,

    /// Enable Rust conversion logs.
    #[arg(env = "MADARA_RUST_EXEC_CONVERSION_LOG", long, default_value_t = false)]
    #[serde(default)]
    pub conversion_log: bool,

    /// Enable hash aggregation logs.
    #[arg(env = "MADARA_RUST_EXEC_HASH_AGG_LOGS", long, default_value_t = false)]
    #[serde(default)]
    pub hash_agg_logs: bool,

    /// Enable storage aggregation logs.
    #[arg(env = "MADARA_RUST_EXEC_STORAGE_AGG_LOGS", long, default_value_t = false)]
    #[serde(default)]
    pub storage_agg_logs: bool,

    /// Ignore fee mismatches in Rust-vs-Blockifier transaction comparison.
    #[arg(env = "MADARA_RUST_EXEC_IGNORE_FEE_MISMATCH", long, default_value_t = false)]
    #[serde(default)]
    pub ignore_fee_mismatch: bool,

    /// Select the nearest settle_trade_v3 bouncer profile by position count.
    #[arg(
        env = "MADARA_RUST_EXEC_SETTLE_TRADE_V3_POSITIONS",
        long,
        value_parser = clap::value_parser!(u16).range(1..=150)
    )]
    #[serde(default)]
    pub settle_trade_v3_positions: Option<u16>,
}

impl Default for RustExecParams {
    fn default() -> Self {
        Self {
            executor_addresses: default_rust_exec_executor_addresses(),
            batch_size: default_rust_exec_batch_size(),
            blockifier_batch_size: default_rust_exec_blockifier_batch_size(),
            execution_log: false,
            execution_log_inner: false,
            tx_diff_log: false,
            debug_block: None,
            inner_timing_log: false,
            ctx_cache: true,
            pedersen_cache: true,
            precomputed_sn_keccak: false,
            conversion_log: false,
            hash_agg_logs: false,
            storage_agg_logs: false,
            ignore_fee_mismatch: false,
            settle_trade_v3_positions: None,
        }
    }
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Deserialize, Serialize, PartialEq, Eq)]
pub enum StartupExecutionModeParam {
    Mixed,
    BlockifierOnly,
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

    /// Rust execution configuration.
    #[clap(flatten)]
    #[serde(default)]
    pub rust_exec: RustExecParams,

    /// Enable replay mode for block production.
    ///
    /// In replay mode, configured replay boundaries are used to constrain batching/execution so
    /// transaction ingestion does not cross source block boundaries.
    #[arg(env = "MADARA_REPLAY_MODE", long, default_value_t = false)]
    pub replay_mode: bool,

    /// Create this number of contracts in the genesis block for the devnet configuration.
    #[arg(env = "MADARA_DEVNET_CONTRACTS", long, default_value_t = 10)]
    pub devnet_contracts: u64,

    /// Startup execution mode for the ExecutionBox pipeline.
    ///
    /// `mixed`: after startup recovery, automatically switch to mixed (Rust + Blockifier) execution.
    /// `blockifier-only`: remain in blockifier-only mode until manual enable via admin RPC.
    #[arg(
        env = "MADARA_STARTUP_EXECUTION_MODE",
        long,
        default_value_t = StartupExecutionModeParam::BlockifierOnly,
        value_enum
    )]
    pub startup_execution_mode: StartupExecutionModeParam,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rstest::rstest;

    #[rstest]
    #[case::default_disabled(vec!["madara"], false)]
    #[case::enabled(vec!["madara", "--replay-mode"], true)]
    fn block_production_params_parse_replay_mode(#[case] args: Vec<&str>, #[case] expected: bool) {
        let params = BlockProductionParams::try_parse_from(args).expect("arguments should parse");
        assert_eq!(params.replay_mode, expected);
    }

    #[rstest]
    #[case::comma_delimited(
        vec!["madara", "--rust-exec-executor-addresses", "0x1,2,0x03"],
        vec!["0x1", "0x2", "0x03"]
    )]
    #[case::repeated_flag(
        vec![
            "madara",
            "--rust-exec-executor-addresses",
            "0x1",
            "--rust-exec-executor-addresses",
            "0x2"
        ],
        vec!["0x1", "0x2"]
    )]
    fn block_production_params_parse_rust_exec_executor_addresses(
        #[case] args: Vec<&str>,
        #[case] expected: Vec<&str>,
    ) {
        let params = BlockProductionParams::try_parse_from(args).expect("arguments should parse");
        assert_eq!(params.rust_exec.executor_addresses, expected.into_iter().map(str::to_string).collect::<Vec<_>>());
    }

    #[test]
    fn block_production_params_reject_invalid_rust_exec_executor_address() {
        let err = BlockProductionParams::try_parse_from(["madara", "--rust-exec-executor-addresses", "not-a-felt"])
            .expect_err("invalid felt must be rejected");
        assert!(err.to_string().contains("invalid felt hex"));
    }

    #[test]
    fn block_production_params_parse_rust_exec_cache_flags() {
        let params = BlockProductionParams::try_parse_from([
            "madara",
            "--rust-exec-ctx-cache=false",
            "--rust-exec-pedersen-cache=false",
            "--rust-exec-precomputed-sn-keccak",
        ])
        .expect("arguments should parse");

        assert!(!params.rust_exec.ctx_cache);
        assert!(!params.rust_exec.pedersen_cache);
        assert!(params.rust_exec.precomputed_sn_keccak);
    }
}
