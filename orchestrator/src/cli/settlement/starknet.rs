use clap::Args;
use url::Url;

#[derive(Debug, Clone, Args)]
// Note: we intentionally do not use requires_all here because env vars can populate
// fields even when --settle-on-starknet is not passed, causing clap to incorrectly
// demand all fields. Validation is done in TryFrom<RunCmd> for SettlementConfig
// (see src/types/params/settlement.rs).
pub struct StarknetSettlementCliArgs {
    /// Use the Starknet settlement layer.
    #[arg(long)]
    pub settle_on_starknet: bool,

    /// The URL of the Ethereum RPC node.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_SETTLEMENT_RPC_URL", long)]
    pub starknet_rpc_url: Option<Url>,

    /// The private key of the Ethereum account.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_PRIVATE_KEY", long)]
    pub starknet_private_key: Option<String>,

    /// The address of the Starknet account.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_ACCOUNT_ADDRESS", long)]
    pub starknet_account_address: Option<String>,

    /// The address of the Cairo core contract.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_CAIRO_CORE_CONTRACT_ADDRESS", long)]
    pub starknet_cairo_core_contract_address: Option<String>,

    /// The number of seconds to wait for finality.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_FINALITY_RETRY_WAIT_IN_SECS", long)]
    pub starknet_finality_retry_wait_in_secs: Option<u64>,
}
