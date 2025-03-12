use clap::Args;
use url::Url;

#[derive(Debug, Clone, Args)]
#[group(requires_all = ["rpc_for_snos"])]
pub struct SNOSCliArgs {
    /// The RPC URL for SNOS.
    #[arg(env = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS", long)]
    pub rpc_for_snos: Url,
}

#[derive(Debug, Clone)]
pub struct SNOSParams {
    pub rpc_for_snos: Url,
}
