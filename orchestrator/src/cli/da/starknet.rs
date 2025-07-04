use clap::Args;
use url::Url;

/// Parameters used to config Starknet.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["starknet_da_rpc_url"])]
pub struct StarknetDaCliArgs {
    /// Use the Starknet DA layer.
    #[arg(long)]
    pub da_on_starknet: bool,

    /// The RPC URL of the Starknet node.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_DA_RPC_URL", long)]
    pub starknet_da_rpc_url: Option<Url>,
}
