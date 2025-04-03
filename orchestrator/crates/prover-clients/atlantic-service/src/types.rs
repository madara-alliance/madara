use serde::{Deserialize, Serialize};
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticAddJobResponse {
    pub atlantic_query_id: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct AtlanticGetProofResponse {
    pub code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticGetStatusResponse {
    pub atlantic_query: AtlanticQuery,
    pub metadata_urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticQuery {
    pub id: String,
    pub external_id: Option<String>,
    pub transaction_id: Option<String>,
    pub status: AtlanticQueryStatus,
    pub step: Option<AtlanticQueryStep>,
    pub program_hash: Option<String>,
    pub integrity_fact_hash: Option<String>,
    pub sharp_fact_hash: Option<String>,
    pub layout: Option<String>,
    pub is_fact_mocked: Option<bool>,
    pub chain: Option<AtlanticChain>,
    pub job_size: Option<AtlanticJobSize>,
    pub declared_job_size: Option<AtlanticJobSize>,
    pub cairo_vm: Option<AtlanticCairoVm>,
    pub cairo_version: Option<AtlanticCairoVersion>,
    pub steps: Vec<AtlanticQueryStep>,
    pub error_reason: Option<String>,
    pub submitted_by_client: String,
    pub project_id: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub result: Option<String>,
    pub network: Option<String>,
    pub client: AtlanticClient,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticClient {
    pub client_id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub is_email_verified: Option<bool>,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryStatus {
    Received,
    InProgress,
    Done,
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AtlanticChain {
    L1,
    L2,
    OFFCHAIN,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AtlanticJobSize {
    S,
    M,
    L,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AtlanticCairoVm {
    Rust,
    Python,
}

impl AtlanticCairoVm {
    pub fn as_str(&self) -> String {
        serde_json::to_string(self).unwrap().trim_matches('"').to_string()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AtlanticCairoVersion {
    Cairo0,
    Cairo1,
}

impl AtlanticCairoVersion {
    pub fn as_str(&self) -> String {
        serde_json::to_string(self).unwrap().trim_matches('"').to_string()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryStep {
    TraceGeneration,
    ProofGeneration,
    ProofVerification,
    ProofVerificationOnL1,
    ProofVerificationOnL2,
    ProofGenerationAndVerification,
    #[cfg(feature = "testing")]
    FactHashRegistration,
}

impl AtlanticQueryStep {
    pub fn as_str(&self) -> String {
        serde_json::to_string(self).unwrap().trim_matches('"').to_string()
    }
}
