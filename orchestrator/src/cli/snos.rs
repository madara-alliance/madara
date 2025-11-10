use clap::Args;
use std::path::PathBuf;
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

    /// Path to a JSON file containing versioned constants to override the default Starknet constants.
    /// By default, versioned constants are picked from the official Starknet constants loaded in blockifier.
    /// Use this argument to override those defaults with custom versioned constants from a file.
    #[arg(env = "MADARA_ORCHESTRATOR_VERSIONED_CONSTANTS_PATH", long, required = false)]
    pub versioned_constants_path: Option<PathBuf>,
}
