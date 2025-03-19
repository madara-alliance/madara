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
    pub metadata_urls: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticQuery {
    pub id: String,
    pub external_id: Option<String>,
    pub transaction_id: Option<String>,
    pub submitted_by_client: String,
    pub status: AtlanticQueryStatus,
    pub step: Option<AtlanticQueryStep>,
    pub program_hash: Option<String>,
    pub integrity_fact_hash: Option<String>,
    pub sharp_fact_hash: Option<String>,
    pub layout: Option<String>,
    pub is_fact_mocked: Option<bool>,
    pub chain: Option<String>,
    pub cairo_vm: Option<String>,
    pub cairo_version: Option<String>,
    pub steps: Vec<AtlanticQueryStep>,
    pub result: Option<String>,
    pub network: Option<String>,
    pub error_reason: Option<String>,
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
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticQueryStep {
    TraceGeneration,
    ProofVerificationOnL1,
    ProofVerificationOnL2,
    ProofGenerationAndVerification,
}
