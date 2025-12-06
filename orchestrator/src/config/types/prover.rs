use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProverConfig {
    /// Prover type: atlantic or sharp
    #[serde(rename = "type")]
    pub prover_type: ProverType,

    /// Layout configuration
    pub layout: ProverLayoutConfig,

    /// Network for prover (optional - uses settlement network if null)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,

    /// RPC URL for prover (optional - uses settlement RPC if null)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<Url>,

    /// Atlantic-specific config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atlantic: Option<AtlanticProverConfig>,

    /// Sharp-specific config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharp: Option<SharpProverConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProverType {
    Atlantic,
    Sharp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProverLayoutConfig {
    #[serde(default = "default_snos_layout")]
    pub snos: String,

    #[serde(default = "default_prover_layout")]
    pub prover: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlanticProverConfig {
    pub api_key: String,
    pub service_url: Url,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock_fact_hash: Option<String>,

    #[serde(default = "default_prover_type")]
    pub prover_type: String,

    pub verifier_contract_address: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cairo_verifier_program_hash: Option<String>,

    #[serde(default = "default_cairo_vm")]
    pub cairo_vm: String,

    #[serde(default = "default_result")]
    pub result: String,

    pub atlantic_network: AtlanticNetwork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AtlanticNetwork {
    Testnet,
    Mainnet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharpProverConfig {
    pub customer_id: String,
    pub url: Url,
    pub user_crt: String,
    pub user_key: String,
    pub rpc_node_url: Url,
    pub server_crt: String,

    #[serde(default = "default_sharp_layout")]
    pub proof_layout: String,

    pub gps_verifier_contract_address: String,
    pub settlement_layer: String,
}

fn default_snos_layout() -> String {
    "all_cairo".to_string()
}

fn default_prover_layout() -> String {
    "dynamic".to_string()
}

fn default_prover_type() -> String {
    "stone".to_string()
}

fn default_cairo_vm() -> String {
    "rust".to_string()
}

fn default_result() -> String {
    "proof-generation".to_string()
}

fn default_sharp_layout() -> String {
    "small".to_string()
}

impl ProverConfig {
    pub fn resolve_network_and_rpc(&mut self, settlement_network: &str, settlement_rpc: &Url) {
        if self.network.is_none() {
            tracing::info!("Prover network not specified, using settlement network: {}", settlement_network);
            self.network = Some(settlement_network.to_string());
        }

        if self.rpc_url.is_none() {
            tracing::info!("Prover RPC not specified, using settlement RPC: {}", settlement_rpc);
            self.rpc_url = Some(settlement_rpc.clone());
        }
    }
}
