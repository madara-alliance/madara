use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::types::{
    AtlanticAddJobResponse, AtlanticCairoVersion, AtlanticCairoVm, AtlanticChain, AtlanticClient,
    AtlanticGetStatusResponse, AtlanticJobSize, AtlanticQuery, AtlanticQueryStatus, AtlanticQueryStep, MockJobData,
    MockServerConfig,
};

#[derive(Clone)]
pub struct MockAtlanticState {
    pub jobs: Arc<RwLock<HashMap<String, MockJobData>>>,
    pub config: MockServerConfig,
}

#[derive(Debug, Deserialize)]
pub struct AddJobQuery {
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProofQuery {
    task_id: String,
}

impl MockAtlanticState {
    pub fn new(config: MockServerConfig) -> Self {
        Self { jobs: Arc::new(RwLock::new(HashMap::new())), config }
    }

    async fn create_mock_job(&self, job_id: String, layout: Option<String>, network: Option<String>) -> MockJobData {
        let now = Utc::now();

        // Use realistic data for the dynamic layout job
        let query = if job_id == "01JXMTC7TZMSNDTJ88212KTH7W" {
            AtlanticQuery {
                id: job_id.clone(),
                external_id: Some("".to_string()),
                transaction_id: Some("01JXMTCJNPFPDW84NNVPHQYZ7H".to_string()),
                status: AtlanticQueryStatus::Done,
                step: Some(AtlanticQueryStep::ProofGeneration),
                program_hash: Some("0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d".to_string()),
                integrity_fact_hash: Some(
                    "0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4".to_string(),
                ),
                sharp_fact_hash: Some("0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123".to_string()),
                layout: Some("dynamic".to_string()),
                is_fact_mocked: None,
                chain: Some(AtlanticChain::OFFCHAIN),
                job_size: Some(AtlanticJobSize::S),
                declared_job_size: Some(AtlanticJobSize::S),
                cairo_vm: Some(AtlanticCairoVm::Rust),
                cairo_version: Some(AtlanticCairoVersion::Cairo0),
                steps: vec![AtlanticQueryStep::TraceAndMetadataGeneration, AtlanticQueryStep::ProofGeneration],
                error_reason: None,
                submitted_by_client: "01JT14WV07E44PE11ZWWCED6BG".to_string(),
                project_id: "01JNFXEF9JFBQFCKXDANCBE5CN".to_string(),
                created_at: "2025-06-13T14:16:24.771Z".to_string(),
                completed_at: Some("2025-06-13T14:17:48.204Z".to_string()),
                result: Some(AtlanticQueryStep::ProofGeneration),
                network: Some("TESTNET".to_string()),
                client: AtlanticClient {
                    client_id: Some("01JT14WV07E44PE11ZWWCED6BG".to_string()),
                    name: Some("itsparser".to_string()),
                    email: Some("itsparser@gmail.com".to_string()),
                    is_email_verified: Some(true),
                    image: Some("https://avatars.githubusercontent.com/u/13918750?v=4".to_string()),
                },
            }
        } else {
            // Default mock data for other job IDs
            AtlanticQuery {
                id: job_id.clone(),
                external_id: None,
                transaction_id: None,
                status: AtlanticQueryStatus::Received,
                step: Some(AtlanticQueryStep::TraceGeneration),
                program_hash: Some("0x123456789abcdef".to_string()),
                integrity_fact_hash: Some("0xfedcba9876543210".to_string()),
                sharp_fact_hash: Some("0xabcdef1234567890".to_string()),
                layout,
                is_fact_mocked: Some(true),
                chain: Some(AtlanticChain::L1),
                job_size: Some(AtlanticJobSize::S),
                declared_job_size: Some(AtlanticJobSize::S),
                cairo_vm: Some(AtlanticCairoVm::Rust),
                cairo_version: Some(AtlanticCairoVersion::Cairo0),
                steps: vec![AtlanticQueryStep::TraceGeneration],
                error_reason: None,
                submitted_by_client: "mock-client".to_string(),
                project_id: "mock-project".to_string(),
                created_at: now.to_rfc3339(),
                completed_at: None,
                result: None,
                network,
                client: AtlanticClient {
                    client_id: Some("mock-client-id".to_string()),
                    name: Some("Mock Client".to_string()),
                    email: Some("mock@example.com".to_string()),
                    is_email_verified: Some(true),
                    image: None,
                },
            }
        };

        // Set proof data immediately for the realistic job since it's already completed
        let proof_data = if job_id == "01JXMTC7TZMSNDTJ88212KTH7W" {
            Some(r#"{
                        "config": {
                            "cairo_version": "cairo0",
                            "layout": "dynamic",
                            "memory_layout": "standard"
                        },
                        "proof": {
                            "proof_a": ["0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4", "0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d"],
                            "proof_b": [["0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123", "0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4"], ["0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d", "0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123"]],
                            "proof_c": ["0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4", "0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d"],
                            "public_inputs": ["0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123", "0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4"]
                        },
                        "fact_hash": "0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123",
                        "verification_result": "VALID"
                    }"#.to_string())
        } else {
            None
        };

        MockJobData { query, proof_data, created_at: now }
    }

    async fn update_job_status(&self, job_id: &str) {
        // Skip dynamic status updates for the realistic job - it's already completed
        if job_id == "01JXMTC7TZMSNDTJ88212KTH7W" {
            return;
        }

        let mut jobs = self.jobs.write().await;
        if let Some(job_data) = jobs.get_mut(job_id) {
            let elapsed = Utc::now().signed_duration_since(job_data.created_at);

            if elapsed.num_milliseconds() > self.config.completion_delay_ms as i64 {
                if self.config.simulate_failures && rand::random::<f32>() < self.config.failure_rate {
                    job_data.query.status = AtlanticQueryStatus::Failed;
                    job_data.query.error_reason = Some("Simulated failure".to_string());
                } else {
                    job_data.query.status = AtlanticQueryStatus::Done;
                    job_data.query.step = Some(AtlanticQueryStep::ProofVerificationOnL1);
                    job_data.query.completed_at = Some(Utc::now().to_rfc3339());

                    // Add mock proof data - use realistic data for the dynamic layout job
                    let proof_data = if job_id == "01JXMTC7TZMSNDTJ88212KTH7W" {
                        r#"{
                        "config": {
                            "cairo_version": "cairo0",
                            "layout": "dynamic",
                            "memory_layout": "standard"
                        },
                        "proof": {
                            "proof_a": ["0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4", "0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d"],
                            "proof_b": [["0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123", "0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4"], ["0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d", "0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123"]],
                            "proof_c": ["0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4", "0x54d3603ed14fb897d0925c48f26330ea9950bd4ca95746dad4f7f09febffe0d"],
                            "public_inputs": ["0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123", "0x1362a28327fd37cc0c430f427208d530fbde13465a975df82339f070d2dafd4"]
                        },
                        "fact_hash": "0x19acd3ad63c7ea36e23a869c40d721e4963e9fc2add5521149be462e10492123",
                        "verification_result": "VALID"
                    }"#
                    } else {
                        r#"{
                        "config": {
                            "cairo_version": "0.12.0",
                            "layout": "dynamic",
                            "memory_layout": "standard"
                        },
                        "proof": {
                            "proof_a": ["0x123...", "0x456..."],
                            "proof_b": [["0x789...", "0xabc..."], ["0xdef...", "0x012..."]],
                            "proof_c": ["0x345...", "0x678..."],
                            "public_inputs": ["0x90ab...", "0xcdef..."]
                        },
                        "fact_hash": "0xfedcba9876543210abcdef1234567890",
                        "verification_result": "VALID"
                    }"#
                    };
                    job_data.proof_data = Some(proof_data.to_string());
                }
            } else if elapsed.num_milliseconds() > self.config.processing_delay_ms as i64 {
                job_data.query.status = AtlanticQueryStatus::InProgress;
                job_data.query.step = Some(AtlanticQueryStep::ProofGeneration);
            }
        }
    }
}

pub async fn add_job_handler(
    State(state): State<MockAtlanticState>,
    Query(query): Query<AddJobQuery>,
    mut multipart: Multipart,
) -> Result<Json<AtlanticAddJobResponse>, StatusCode> {
    info!("Received add_job request with API key: {:?}", query.api_key);

    let mut layout = None;
    let mut network = None;
    let mut declared_job_size = None;
    let mut cairo_version = None;
    let mut cairo_vm = None;
    let mut result_type = None;

    while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
        match field.name() {
            Some("layout") => {
                layout = Some(field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            Some("network") => {
                network = Some(field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            Some("declaredJobSize") => {
                declared_job_size = Some(field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            Some("cairoVersion") => {
                cairo_version = Some(field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            Some("cairoVm") => {
                cairo_vm = Some(field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            Some("result") => {
                result_type = Some(field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            Some("pieFile") | Some("inputFile") | Some("programFile") => {
                let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
                debug!("Received file data of {} bytes", data.len());
            }
            _ => {
                let name = field.name().map(|s| s.to_string());
                let value = field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?;
                if let Some(name) = name {
                    debug!("Received form field: {} = {}", name, value);
                }
            }
        }
    }

    // Determine job ID based on layout
    let job_id = match layout.as_deref() {
        Some("recursive_with_poseidon") => "01JXMXQAX4KNNSQDKDZTSHG8FC".to_string(),
        Some("dynamic") => "01JXMTC7TZMSNDTJ88212KTH7W".to_string(),
        _ => {
            // Default to dynamic layout ID for other layouts
            debug!("Using default job ID for layout: {:?}", layout);
            "01JXMTC7TZMSNDTJ88212KTH7W".to_string()
        }
    };

    debug!("Creating job with layout: {:?}, network: {:?}, job_size: {:?}, cairo_version: {:?}, cairo_vm: {:?}, result_type: {:?}, job_id: {}", 
           layout, network, declared_job_size, cairo_version, cairo_vm, result_type, job_id);

    let job_data = state.create_mock_job(job_id.clone(), layout.clone(), network).await;
    let job_layout = job_data.query.layout.clone();
    state.jobs.write().await.insert(job_id.clone(), job_data);

    info!("Created mock job with ID: {} for layout: {:?}", job_id, job_layout);

    Ok(Json(AtlanticAddJobResponse { atlantic_query_id: job_id }))
}

pub async fn get_job_status_handler(
    State(state): State<MockAtlanticState>,
    Path(job_id): Path<String>,
) -> Result<Json<AtlanticGetStatusResponse>, StatusCode> {
    debug!("Received get_job_status request for job: {}", job_id);

    state.update_job_status(&job_id).await;

    let jobs = state.jobs.read().await;
    match jobs.get(&job_id) {
        Some(job_data) => {
            info!("Returning status for job {}: {:?}", job_id, job_data.query.status);
            Ok(Json(AtlanticGetStatusResponse {
                atlantic_query: job_data.query.clone(),
                metadata_urls: vec![format!("https://mock-atlantic.example.com/metadata/{}", job_id)],
            }))
        }
        None => {
            warn!("Job not found: {}", job_id);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

pub async fn get_proof_handler(
    State(state): State<MockAtlanticState>,
    Path(task_id): Path<String>,
) -> Result<String, StatusCode> {
    debug!("Received get_proof request for task: {}", task_id);

    let jobs = state.jobs.read().await;
    match jobs.get(&task_id) {
        Some(job_data) => {
            if job_data.query.status == AtlanticQueryStatus::Done {
                if let Some(ref proof_data) = job_data.proof_data {
                    info!("Returning proof data for task: {}", task_id);
                    Ok(proof_data.clone())
                } else {
                    warn!("Proof data not available for task: {}", task_id);
                    Err(StatusCode::NOT_FOUND)
                }
            } else {
                warn!("Task not completed yet: {}", task_id);
                Err(StatusCode::NOT_FOUND)
            }
        }
        None => {
            warn!("Task not found: {}", task_id);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

pub async fn health_check() -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({
        "status": "healthy",
        "service": "mock-atlantic-server",
        "timestamp": Utc::now().to_rfc3339()
    })))
}

pub fn create_router(state: MockAtlanticState) -> Router {
    Router::new()
        .route("/atlantic-query", post(add_job_handler))
        .route("/atlantic-query/:job_id", get(get_job_status_handler))
        .route("/queries/:task_id/proof.json", get(get_proof_handler))
        .route("/health", get(health_check))
        .with_state(state)
}
