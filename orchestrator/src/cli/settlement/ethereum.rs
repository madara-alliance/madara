use clap::Args;
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

    /// The amount of time in seconds to wait for state update txns
    /// Doesn't require an env variable
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_FINALITY_RETRY_WAIT_IN_SECS", long, default_value = "60")]
    pub ethereum_finality_retry_wait_in_secs: Option<u64>,

    /// Maximum time to wait for a submitted Ethereum state-update transaction to finalize
    /// before submitting a same-nonce fee-bump replacement.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_TX_CONFIRMATION_TIMEOUT_SECS", long, default_value = "300")]
    pub ethereum_tx_confirmation_timeout_secs: u64,

    /// Maximum number of same-nonce fee-bump replacements for a state-update transaction.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_MAX_FEE_BUMPS", long, default_value = "2")]
    pub ethereum_max_fee_bumps: u64,

    /// Disable PeerDAS (PeerDAS is a feature introduced in Fusaka upgrade which changes the way we settle on Ethereum).
    /// https://ethereum.org/roadmap/fusaka
    /// https://notes.ethereum.org/@fradamt/das-fork-choice
    /// Whether settling on Ethereum mainnet (true) or Sepolia testnet (false).
    /// Mainnet uses blob proofs (pre-Fusaka), Sepolia uses cell proofs (post-Fusaka).
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_DISABLE_PEERDAS", long, default_value = "false")]
    pub disable_peerdas: bool,
}
