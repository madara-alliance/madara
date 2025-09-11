use clap::Args;
use orchestrator_atlantic_service::types::{AtlanticCairoVm, AtlanticQueryStep};
use url::Url;

/// Parameters used to config Atlantic.
#[derive(Debug, Clone, Args)]
pub struct AtlanticCliArgs {
    /// Use the Atlantic prover.
    #[arg(long)]
    pub atlantic: bool,

    /// The API key for the Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_API_KEY", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_api_key: Option<String>,

    /// The URL of the Atlantic server.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_service_url: Option<Url>,

    /// The URL of the Atlantic RPC node.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_rpc_node_url: Option<Url>,

    /// Enable mock mode for Atlantic prover (set to "true" to enable).
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_MOCK_FACT_HASH", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_mock_fact_hash: Option<String>,

    /// The type of prover to use.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_PROVER_TYPE", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_prover_type: Option<String>,

    /// The settlement layer for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_SETTLEMENT_LAYER", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_settlement_layer: Option<String>,

    /// The verifier contract address for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS", long)]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_verifier_contract_address: Option<String>,

    /// The verifier contract address for Atlantic.
    #[arg(env = "MADARA_ORCHESTRATOR_CAIRO_V0_VERIFIER_PROGRAM_HASH", long)]
    pub cairo_verifier_program_hash: Option<String>,

    /// The cairo vm for atlantic
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_CAIRO_VM", long, default_value = "rust")]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_verifier_cairo_vm: Option<AtlanticCairoVm>,

    /// The type of job atlantic should process
    #[arg(env = "MADARA_ORCHESTRATOR_ATLANTIC_RESULT", long, default_value = "proof-generation")]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_verifier_result: Option<AtlanticQueryStep>,

    /// Network being used for the prover.
    #[arg(
        env = "MADARA_ORCHESTRATOR_ATLANTIC_NETWORK", 
        long,
        value_parser = ["MAINNET", "TESTNET"]
    )]
    #[arg(required_if_eq("atlantic", "true"))]
    pub atlantic_network: Option<String>,
}
