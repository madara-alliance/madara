use std::path::PathBuf;

use clap::Args;
use url::Url;

/// Parameters used to config Sharp.
///
/// mTLS cert material is **file-only**: pass paths via
/// `MADARA_ORCHESTRATOR_SHARP_USER_CRT_FILE` etc. The file content is read as
/// raw PEM (no base64 wrapping). This matches how k8s CSI drivers / Vault /
/// AWS Secrets Manager mount secrets, and avoids leaking cert material via
/// `ps`, `/proc/<pid>/environ`, CI logs, or crash dumps.
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

    /// Path to the PEM-encoded client certificate.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_USER_CRT_FILE", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_user_crt_file: Option<PathBuf>,

    /// Path to the PEM-encoded client private key (PKCS#8 or PKCS#1).
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_USER_KEY_FILE", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_user_key_file: Option<PathBuf>,

    /// The RPC node URL for Sharp.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_rpc_node_url: Option<Url>,

    /// Path to the PEM-encoded server certificate to trust.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_SERVER_CRT_FILE", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub sharp_server_crt_file: Option<PathBuf>,

    /// The GPS verifier contract address.
    #[arg(env = "MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS", long)]
    #[arg(required_if_eq("prover", "sharp"))]
    pub gps_verifier_contract_address: Option<String>,

    /// Settlement layer for Sharp. Defaults to "ethereum" since SHARP is L2-only.
    #[arg(env = "MADARA_ORCHESTRATOR_SHARP_SETTLEMENT_LAYER", long, default_value = "ethereum")]
    pub sharp_settlement_layer: Option<String>,
}
