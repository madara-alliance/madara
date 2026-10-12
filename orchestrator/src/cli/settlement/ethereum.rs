use clap::Args;
use orchestrator_ethereum_settlement_client::{
    DEFAULT_L2_STATE_UPDATE_MAX_FEE_WEI, DEFAULT_REQUIRED_BLOCK_CONFIRMATIONS,
};
use url::Url;

#[derive(Debug, Clone, Args)]
// Note: we intentionally do not use requires_all here because env vars can populate
// fields even when --settle-on-ethereum is not passed, causing clap to incorrectly
// demand all fields. Validation is done in TryFrom<RunCmd> for SettlementConfig
// (see src/types/params/settlement.rs).
pub struct EthereumSettlementCliArgs {
    /// Use the Ethereum settlement layer.
    #[arg(long)]
    pub settle_on_ethereum: bool,

    /// The URL of the Ethereum RPC node.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", long)]
    pub ethereum_rpc_url: Option<Url>,

    /// The private key of the Ethereum account.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY", long)]
    pub ethereum_private_key: Option<String>,

    /// The address of the L1 core contract.
    #[arg(env = "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS", long)]
    pub l1_core_contract_address: Option<String>,

    /// The address of the Starknet operator.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS", long)]
    pub starknet_operator_address: Option<String>,

    /// Seconds between L1 receipt/confirmation checks. This does not change the required confirmation depth.
    #[arg(
        env = "MADARA_ORCHESTRATOR_ETHEREUM_FINALITY_RETRY_WAIT_IN_SECS",
        long,
        default_value = "60",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub ethereum_finality_retry_wait_in_secs: Option<u64>,

    /// Additional L1 blocks required after the inclusion block. Zero accepts successful inclusion immediately.
    /// This is a confirmation-depth policy, not Ethereum consensus finality.
    #[arg(
        env = "MADARA_ORCHESTRATOR_ETHEREUM_REQUIRED_BLOCK_CONFIRMATIONS",
        long,
        default_value_t = DEFAULT_REQUIRED_BLOCK_CONFIRMATIONS
    )]
    pub ethereum_required_block_confirmations: u64,

    /// Maximum time to wait for a submitted Ethereum state-update transaction to finalize
    /// before submitting a same-nonce fee-bump replacement.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_TX_CONFIRMATION_TIMEOUT_SECS", long, default_value = "300")]
    pub ethereum_tx_confirmation_timeout_secs: u64,

    /// Maximum number of same-nonce fee-bump replacements for a state-update transaction.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_MAX_FEE_BUMPS", long, default_value = "2")]
    pub ethereum_max_fee_bumps: u64,

    /// Maximum signed fee liability in wei for an L2 Ethereum state-update transaction, including six blobs.
    /// The conservative default can reject settlements during high-fee periods; raise it when liveness takes priority.
    #[arg(
        env = "MADARA_ORCHESTRATOR_ETHEREUM_L2_STATE_UPDATE_MAX_FEE_WEI",
        long,
        default_value_t = DEFAULT_L2_STATE_UPDATE_MAX_FEE_WEI
    )]
    pub ethereum_l2_state_update_max_fee_wei: u128,

    /// Disable PeerDAS (PeerDAS is a feature introduced in Fusaka upgrade which changes the way we settle on Ethereum).
    /// https://ethereum.org/roadmap/fusaka
    /// https://notes.ethereum.org/@fradamt/das-fork-choice
    /// Whether settling on Ethereum mainnet (true) or Sepolia testnet (false).
    /// Mainnet uses blob proofs (pre-Fusaka), Sepolia uses cell proofs (post-Fusaka).
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_DISABLE_PEERDAS", long, default_value = "false")]
    pub disable_peerdas: bool,
}

#[cfg(test)]
mod settlement_wait_tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches, Parser};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        ethereum: EthereumSettlementCliArgs,
    }

    fn parse_without_env<const N: usize>(args: [&str; N]) -> clap::error::Result<TestCli> {
        let matches = TestCli::command().mut_args(|arg| arg.env(None::<&'static str>)).try_get_matches_from(args)?;
        TestCli::from_arg_matches(&matches)
    }

    #[test]
    fn preserves_defaults_and_accepts_shorter_waits() {
        let defaults = parse_without_env(["test"]).unwrap().ethereum;
        assert_eq!(defaults.ethereum_finality_retry_wait_in_secs, Some(60));
        assert_eq!(defaults.ethereum_required_block_confirmations, 3);
        let configured = parse_without_env([
            "test",
            "--ethereum-finality-retry-wait-in-secs",
            "5",
            "--ethereum-required-block-confirmations",
            "0",
        ])
        .unwrap()
        .ethereum;
        assert_eq!(configured.ethereum_finality_retry_wait_in_secs, Some(5));
        assert_eq!(configured.ethereum_required_block_confirmations, 0);
        assert!(parse_without_env(["test", "--ethereum-finality-retry-wait-in-secs", "0"]).is_err());
    }

    #[test]
    fn exposes_both_wait_settings_through_env() {
        let command = TestCli::command();
        for (id, env) in [
            ("ethereum_finality_retry_wait_in_secs", "MADARA_ORCHESTRATOR_ETHEREUM_FINALITY_RETRY_WAIT_IN_SECS"),
            ("ethereum_required_block_confirmations", "MADARA_ORCHESTRATOR_ETHEREUM_REQUIRED_BLOCK_CONFIRMATIONS"),
        ] {
            let arg = command.get_arguments().find(|arg| arg.get_id() == id).unwrap();
            assert_eq!(arg.get_env(), Some(std::ffi::OsStr::new(env)));
        }
    }
}
