use clap::Args;
use url::Url;

// Getting the default fee token addresses from the SNOS
use generate_pie::constants::{DEFAULT_SEPOLIA_ETH_FEE_TOKEN, DEFAULT_SEPOLIA_STRK_FEE_TOKEN};

#[derive(Debug, Clone, Args)]
#[group(requires_all = ["rpc_for_snos"])]
pub struct SNOSCliArgs {
    /// Weather to use full output or not
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_FULL_OUTPUT", long, default_value = "false")]
    pub snos_full_output: bool,

    /// The RPC URL for SNOS.
    #[arg(env = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS", long)]
    pub rpc_for_snos: Url,

    /// Address of STRK native fee token
    #[arg(env = "MADARA_ORCHESTRATOR_STRK_NATIVE_FEE_TOKEN_ADDRESS", long, required = false, default_value = DEFAULT_SEPOLIA_STRK_FEE_TOKEN)]
    pub strk_fee_token_address: String,

    /// Address of ETH native fee token
    #[arg(env = "MADARA_ORCHESTRATOR_ETH_NATIVE_FEE_TOKEN_ADDRESS", long, required = false, default_value = DEFAULT_SEPOLIA_ETH_FEE_TOKEN)]
    pub eth_fee_token_address: String,
}
