use std::path::PathBuf;

use blockifier::blockifier_versioned_constants::VersionedConstants;
use clap::Args;
use generate_pie::utils::load_versioned_constants;
use url::Url;

pub(crate) fn parse_constants(path: &str) -> Result<VersionedConstants, String> {
    load_versioned_constants(Some(path))?
        .ok_or_else(|| format!("Failed to load versioned constants from file: {}", PathBuf::from(path).display()))
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
