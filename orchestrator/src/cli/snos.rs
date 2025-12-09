use std::path::PathBuf;

use blockifier::blockifier_versioned_constants::VersionedConstants;
use clap::Args;
use url::Url;

fn parse_constants(path: &str) -> Result<VersionedConstants, String> {
    let path_buf = PathBuf::from(path);
    tracing::debug!(file_path = %path_buf.display(), "Loading versioned constants from file");
    VersionedConstants::from_path(&path_buf).map_err(|e| format!("Failed to load versioned constants from file: {}", e))
}

#[derive(Debug, Clone, Args)]
#[group(requires_all = ["rpc_for_snos"])]
pub struct SNOSCliArgs {
    /// Weather to use full output or not
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_FULL_OUTPUT", long, default_value = "false")]
    pub snos_full_output: bool,

    /// The RPC URL for SNOS.
    #[arg(env = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS", long)]
    pub rpc_for_snos: Url,

    /// Path to a JSON file containing versioned constants to override the default Starknet constants.
    /// By default, versioned constants are picked from the official Starknet constants loaded in blockifier.
    /// Use this argument to override those defaults with custom versioned constants from a file.
    #[arg(env = "MADARA_ORCHESTRATOR_VERSIONED_CONSTANTS_PATH", long, required = false, value_parser = parse_constants)]
    pub versioned_constants: Option<VersionedConstants>,
}
