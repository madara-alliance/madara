use clap::Args;
use url::Url;

/// Parameters used to config Sharp.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["sharp_customer_id", "sharp_url", "sharp_user_crt", "sharp_user_key", "sharp_server_crt", "gps_verifier_contract_address", "sharp_rpc_node_url"])]
pub struct SharpCliArgs {
    /// Use the Sharp prover.
    #[arg(long)]
    pub sharp: bool,

    /// The customer id for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID", long)]
    pub sharp_customer_id: Option<String>,

    /// The URL of the Sharp server.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_URL", long)]
    pub sharp_url: Option<Url>,

    /// The user certificate for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_USER_CRT", long)]
    pub sharp_user_crt: Option<String>,

    /// The user key for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_USER_KEY", long)]
    pub sharp_user_key: Option<String>,

    /// The RPC node URL for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL", long)]
    pub sharp_rpc_node_url: Option<Url>,

    /// The server certificate for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_SERVER_CRT", long)]
    pub sharp_server_crt: Option<String>,

    /// The proof layout for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT", long, default_value = "small")]
    pub sharp_proof_layout: Option<String>,

    // TODO: GPS is a direct dependency of Sharp, hence GPS can be kept in SharpValidatedArgs
    /// The GPS verifier contract address.
    #[arg(env = "MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS", long)]
    pub gps_verifier_contract_address: Option<String>,
}
