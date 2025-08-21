use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// TODO: ideally would want to use the types from the `atlantic` crate in orchestrator
// We currently have to clone these due to lack of common types trait

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticAddJobResponse {
    pub atlantic_query_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticGetStatusResponse {
    pub atlantic_query: AtlanticQuery,
    pub metadata_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub result: Option<AtlanticQueryStep>,
    pub network: Option<String>,
    pub client: AtlanticClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticClient {
    pub client_id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub is_email_verified: Option<bool>,
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryStatus {
    Received,
    InProgress,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AtlanticChain {
    L1,
    L2,
    OFFCHAIN,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AtlanticJobSize {
    S,
    M,
    L,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryStep {
    TraceGeneration,
    ProofGeneration,
    ProofVerification,
    ProofVerificationOnL1,
    ProofVerificationOnL2,
    ProofGenerationAndVerification,
    FactHashRegistration,
    TraceAndMetadataGeneration,
}

impl std::fmt::Display for AtlanticQueryStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap().trim_matches('"'))
    }
}

#[derive(Debug, Clone)]
pub struct MockJobData {
    pub query: AtlanticQuery,
    pub proof_data: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MockServerConfig {
    pub simulate_failures: bool,
    pub processing_delay_ms: u64,
    pub failure_rate: f32, // 0.0 to 1.0
    pub auto_complete_jobs: bool,
    pub completion_delay_ms: u64,
}
