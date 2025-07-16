use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticBucket {
    pub id: String,
    pub external_id: Option<String>,
    pub status: AtlanticBucketStatus,
    pub aggregator_version: AtlanticAggregatorVersion,
    pub node_width: Option<i64>,
    pub leaves: Option<i64>,
    pub chain: AtlanticChain,
    pub project_id: String,
    pub created_by_client: String,
    pub created_at: String,
}

/// This is the response struct for `create` and `close` bucket requests
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticBucketResponse {
    pub atlantic_bucket: AtlanticBucket,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AtlanticQueryBucket {
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
    pub hints: Option<AtlanticHints>,
    pub bucket_id: String,
    pub bucket_job_index: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticGetBucketResponse {
    pub bucket: AtlanticBucket,
    pub queries: Vec<AtlanticQueryBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlanticCreateBucketRequest {
    pub external_id: Option<String>,
    pub node_width: Option<String>,
    pub aggregator_version: AtlanticAggregatorVersion,
    pub aggregator_params: AtlanticAggregatorParams,
}

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
    pub result: Option<AtlanticQueryStep>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AtlanticAggregatorParams {
    pub(crate) use_kzg_da: bool,
    pub(crate) full_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticBucketStatus {
    Open,
    InProgress,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AtlanticAggregatorVersion {
    #[serde(rename = "snos_aggregator_0.13.2")]
    SnosAggregator0_13_2,
    #[serde(rename = "snos_aggregator_0.13.3")]
    SnosAggregator0_13_3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtlanticBucketType {
    Snos,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtlanticHints {
    HerodotusEvmGrower,
    HerodotusSnGrower,
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

#[derive(Debug, Clone, clap::ValueEnum, Deserialize, Serialize)]
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

#[derive(Debug, Clone, clap::ValueEnum, Deserialize, Serialize)]
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
