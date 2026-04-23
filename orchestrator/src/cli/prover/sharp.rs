use clap::Args;
use url::Url;

/// Parameters used to config Sharp.
///
/// Selection of this prover is via the top-level `--prover` arg /
/// `MADARA_ORCHESTRATOR_PROVER=sharp` env. There is no `--sharp` flag.
/// `required_if_eq` hooks here enforce that the sub-envs below are present
/// *at clap parse time* whenever `prover=sharp`. The same fields are then
/// unwrapped in `validate_sharp` for the validated struct, so you get both
/// early and late checks with matching error messages.
#[derive(Debug, Clone, Args)]
pub struct SharpCliArgs {
    /// The customer id for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_customer_id: Option<String>,

    /// The URL of the Sharp server.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_URL", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_url: Option<Url>,

    /// The user certificate for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_USER_CRT", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_user_crt: Option<String>,

    /// The user key for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_USER_KEY", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_user_key: Option<String>,

    /// The RPC node URL for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_rpc_node_url: Option<Url>,

    /// The server certificate for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_SERVER_CRT", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_server_crt: Option<String>,

    /// The proof layout for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT", long, default_value = "small")]
    pub sharp_proof_layout: Option<String>,

    /// The GPS verifier contract address.
    #[arg(env = "MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub gps_verifier_contract_address: Option<String>,

    /// Settlement layer for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_SETTLEMENT_LAYER", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_settlement_layer: Option<String>,
}
