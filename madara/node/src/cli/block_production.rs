use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use std::path::PathBuf;

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

    /// Load Rust execution routing configuration from a YAML/JSON/TOML file.
    ///
    /// The file uses the `RustExecRoutingConfig` field names:
    /// `executor_addresses`, `supported_selectors`, `supported_class_hashes`,
    /// `rust_batch_size`, and `blockifier_batch_size`.
    #[arg(env = "MADARA_RUST_EXEC_ROUTING_CONFIG", long, value_name = "PATH")]
    pub rust_exec_routing_config: Option<PathBuf>,

    /// Override the sender-address whitelist used for Rust execution routing.
    ///
    /// Values are comma-delimited felts.
    #[arg(
        env = "MADARA_RUST_EXEC_EXECUTOR_ADDRESSES",
        long,
        value_delimiter = ',',
        value_parser = parse_hex_felt_arg,
        value_name = "ADDRESS[,ADDRESS...]"
    )]
    pub rust_exec_executor_addresses: Option<Vec<String>>,

    /// Override the selector whitelist used for Rust execution routing.
    ///
    /// Values are comma-delimited felts.
    #[arg(
        env = "MADARA_RUST_EXEC_SUPPORTED_SELECTORS",
        long,
        value_delimiter = ',',
        value_parser = parse_hex_felt_arg,
        value_name = "SELECTOR[,SELECTOR...]"
    )]
    pub rust_exec_supported_selectors: Option<Vec<String>>,

    /// Override the contract class-hash whitelist used for Rust execution routing.
    ///
    /// Values are comma-delimited felts.
    #[arg(
        env = "MADARA_RUST_EXEC_SUPPORTED_CLASS_HASHES",
        long,
        value_delimiter = ',',
        value_parser = parse_hex_felt_arg,
        value_name = "CLASS_HASH[,CLASS_HASH...]"
    )]
    pub rust_exec_supported_class_hashes: Option<Vec<String>>,

    /// Override the maximum Rust-routed transactions picked in one batcher cycle.
    #[arg(
        env = "MADARA_RUST_EXEC_BATCH_SIZE",
        long,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub rust_exec_batch_size_override: Option<u64>,

    /// Override the maximum Blockifier-routed transactions picked in one batcher cycle.
    #[arg(
        env = "MADARA_RUST_EXEC_BLOCKIFIER_BATCH_SIZE",
        long,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub rust_exec_blockifier_batch_size_override: Option<u64>,

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
        vec!["madara", "--rust-exec-supported-class-hashes", "0x1,2,0x03"],
        vec!["0x1", "0x2", "0x03"]
    )]
    #[case::repeated_flag(
        vec![
            "madara",
            "--rust-exec-supported-class-hashes",
            "0x1",
            "--rust-exec-supported-class-hashes",
            "0x2"
        ],
        vec!["0x1", "0x2"]
    )]
    fn block_production_params_parse_rust_exec_supported_class_hashes(
        #[case] args: Vec<&str>,
        #[case] expected: Vec<&str>,
    ) {
        let params = BlockProductionParams::try_parse_from(args).expect("arguments should parse");
        assert_eq!(params.rust_exec_supported_class_hashes, Some(expected.into_iter().map(str::to_string).collect()));
    }

    #[test]
    fn block_production_params_reject_invalid_rust_exec_supported_class_hash() {
        let err = BlockProductionParams::try_parse_from(["madara", "--rust-exec-supported-class-hashes", "not-a-felt"])
            .expect_err("invalid felt must be rejected");
        assert!(err.to_string().contains("invalid felt hex"));
    }

    #[test]
    fn block_production_params_parse_rust_exec_routing_config_path() {
        let params = BlockProductionParams::try_parse_from([
            "madara",
            "--rust-exec-routing-config",
            "configs/rust_exec_routing.yaml",
        ])
        .expect("arguments should parse");

        assert_eq!(params.rust_exec_routing_config, Some(PathBuf::from("configs/rust_exec_routing.yaml")));
    }
}
