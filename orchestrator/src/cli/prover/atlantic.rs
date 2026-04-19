use clap::Args;
use orchestrator_atlantic_service::types::{AtlanticCairoVm, AtlanticQueryStep, AtlanticSharpProver};
use url::Url;

/// Parameters used to config Atlantic.
///
/// Selection of this prover is via the top-level `--prover` arg /
/// `MADARA_ORCHESTRATOR_PROVER=atlantic` env. There is no `--atlantic` flag.
/// `required_if_eq("prover", "atlantic")` on the sub-args below gives clap
/// parse-time enforcement that the Atlantic-specific envs are present when
/// Atlantic is selected; the same checks run again in `validate_atlantic` to
/// build the `AtlanticValidatedArgs` struct.
#[derive(Debug, Clone, Args)]
pub struct AtlanticCliArgs {
    /// The API key for the Atlantic.
    /// Note: not marked required_if_eq because the value can be loaded from a file
    /// via MADARA_ORCHESTRATOR_ATLANTIC_API_KEY_FILE. Validation is done in
    /// TryFrom<RunCmd> for ProverConfig (see src/types/params/prover.rs).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_API_KEY", long)]
    pub atlantic_api_key: Option<String>,

    /// The URL of the Atlantic server.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL", long)]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_service_url: Option<Url>,

    /// The URL of the Atlantic RPC node.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL", long)]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_rpc_node_url: Option<Url>,

    /// Enable mock mode for Atlantic prover (set to "true" to enable).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_MOCK_FACT_HASH", long)]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_mock_fact_hash: Option<String>,

    /// The type of prover to use.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_PROVER_TYPE", long)]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_prover_type: Option<String>,

    /// The settlement layer for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SETTLEMENT_LAYER", long)]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_settlement_layer: Option<String>,

    /// The verifier contract address for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS", long)]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_verifier_contract_address: Option<String>,

    /// The verifier contract address for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_CAIRO_V0_VERIFIER_PROGRAM_HASH", long)]
    pub cairo_verifier_program_hash: Option<String>,

    /// The cairo vm for atlantic
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_CAIRO_VM", long, default_value = Some("rust"))]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_verifier_cairo_vm: Option<AtlanticCairoVm>,

    /// The type of job atlantic should process
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_RESULT", long, default_value = Some("proof-generation"))]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_verifier_result: Option<AtlanticQueryStep>,

    /// Network being used for the prover.
    #[arg(
        env = "MADARA_ORCHESTRATOR_ATLANTIC_NETWORK",
        long,
        value_parser = ["MAINNET", "TESTNET"]
    )]
    #[arg(required_if_eq("prover", "atlantic"))]
    pub atlantic_network: Option<String>,

    /// The SHARP prover backend to use (stone or stwo).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SHARP_PROVER", long, default_value = "stone")]
    pub atlantic_sharp_prover: Option<AtlanticSharpProver>,

    /// The base URL for fetching artifacts from the Atlantic service (e.g., proofs, SNOS outputs).
    #[arg(
        env = "MADARA_ORCHESTRATOR_ATLANTIC_ARTIFACTS_BASE_URL",
        long,
        default_value = "https://storage.googleapis.com/hero-atlantic-bucket"
    )]
    pub atlantic_artifacts_base_url: Option<Url>,
}
