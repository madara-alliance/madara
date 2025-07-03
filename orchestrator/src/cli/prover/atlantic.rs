use clap::Args;
use url::Url;

/// Parameters used to config Atlantic.
#[derive(Debug, Clone, Args)]
pub struct AtlanticCliArgs {
    /// Use the Atlantic prover.
    #[arg(long)]
    pub atlantic: bool,

    /// The API key for the Atlantic (optional for mock mode, defaults to "mock-api-key").
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_API_KEY", long)]
    pub atlantic_api_key: Option<String>,

    /// The URL of the Atlantic server (optional for mock mode, will be overridden to localhost).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL", long)]
    pub atlantic_service_url: Option<Url>,

    /// The URL of the Atlantic RPC node (optional for mock mode, not used in mock mode).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL", long)]
    pub atlantic_rpc_node_url: Option<Url>,

    /// Whether to use mock fact registry (set to "true" for testing, "false" for production).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_MOCK_FACT_HASH", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_mock_fact_hash: Option<String>,

    /// The type of prover to use (optional, defaults to "starkware").
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_PROVER_TYPE", long)]
    pub atlantic_prover_type: Option<String>,

    /// The settlement layer for Atlantic (optional, defaults to "ethereum").
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SETTLEMENT_LAYER", long)]
    pub atlantic_settlement_layer: Option<String>,

    /// The verifier contract address for Atlantic (optional for mock mode, will be hardcoded).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS", long)]
    pub atlantic_verifier_contract_address: Option<String>,

    /// The verifier contract address for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_CAIRO_V0_VERIFIER_PROGRAM_HASH", long)]
    pub cairo_verifier_program_hash: Option<String>,

    /// Network being used for the prover (optional, defaults to "TESTNET").
    #[arg(
        env = "MADARA_ORCHESTRATOR_ATLANTIC_NETWORK", 
        long,
        value_parser = ["MAINNET", "TESTNET"]
    )]
    pub atlantic_network: Option<String>,
}
