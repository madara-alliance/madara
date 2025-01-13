use clap::Args;
use url::Url;

/// Parameters used to config Atlantic.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["atlantic_api_key", "atlantic_service_url", "atlantic_settlement_layer", "atlantic_verifier_contract_address"])]
pub struct AtlanticCliArgs {
    /// Use the Atlantic prover.
    #[arg(long)]
    pub atlantic: bool,

    /// The API key for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_API_KEY", long)]
    pub atlantic_api_key: Option<String>,

    /// The URL of the Atlantic server.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL", long)]
    pub atlantic_service_url: Option<Url>,

    /// The URL of the Atlantic RPC node.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL", long)]
    pub atlantic_rpc_node_url: Option<Url>,

    /// Whether to use mock fact registry.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_MOCK_FACT_HASH", long)]
    pub atlantic_mock_fact_hash: Option<String>,

    /// The type of prover to use.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_PROVER_TYPE", long)]
    pub atlantic_prover_type: Option<String>,

    /// The settlement layer for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SETTLEMENT_LAYER", long)]
    pub atlantic_settlement_layer: Option<String>,

    /// The verifier contract address for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS", long)]
    pub atlantic_verifier_contract_address: Option<String>,
}
