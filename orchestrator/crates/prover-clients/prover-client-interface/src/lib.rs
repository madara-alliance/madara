pub mod http;
pub mod retry;

use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use mockall::automock;
use orchestrator_gps_fact_checker::FactCheckerError;

/// Prover client provides an abstraction over different proving services that do the following:
/// - Accept a task containing Cairo intermediate execution artifacts (in PIE format)
/// - Aggregate multiple tasks and prove the execution (of the bootloader program where PIEs are
///   inputs)
/// - Register the proof onchain (individual proof facts available for each task)
///
/// A common Madara workflow would be a single task per block (SNOS execution result) or per block
/// span (SNAR).
#[automock]
#[async_trait]
pub trait ProverClient: Send + Sync {
    async fn submit_task(&self, task: Task) -> Result<String, ProverClientError>;
    async fn get_task_status(
        &self,
        task: TaskType,
        task_id: &str,
        fact: Option<String>,
    ) -> Result<TaskStatus, ProverClientError>;
    async fn get_proof(&self, task_id: &str) -> Result<String, ProverClientError>;
    async fn submit_l2_query(
        &self,
        task_id: &str,
        fact: &str,
        n_steps: Option<usize>,
    ) -> Result<String, ProverClientError>;

    /// Fetch aggregation artifacts after aggregation status is Succeeded.
    ///
    /// For provers that aggregate remotely (e.g. Atlantic), this fetches CairoPIE, DA segment,
    /// and optionally the proof from the remote service.
    ///
    /// For provers that aggregate locally (e.g. SHARP, Mock), artifacts are already stored by
    /// the handler during `process_job`, so this returns all `None`.
    async fn get_aggregation_artifacts(
        &self,
        external_id: &str,
        include_proof: bool,
    ) -> Result<AggregationArtifacts, ProverClientError>;
}

pub struct CreateJobInfo {
    pub cairo_pie: Box<CairoPie>,
    pub bucket_id: Option<String>,
    pub bucket_job_index: Option<u64>,
    pub num_steps: Option<usize>,
    pub dedup_id: String,
}

/// Information for submitting a pre-built aggregator CairoPIE (SHARP applicative job,
/// or Mock's fact registration).
pub struct ApplicativeJobInfo {
    pub cairo_pie_zip_bytes: bytes::Bytes,
    pub children_cairo_job_keys: Vec<String>,
    /// The fact hash for this aggregator PIE, computed by the handler via `get_fact_info`.
    /// Used by the Mock prover to register on an L1 `MockGpsVerifier`. SHARP ignores it
    /// (SHARP computes the fact server-side).
    pub fact_hash: Option<[u8; 32]>,
}

/// Artifacts returned by a prover after aggregation completes.
///
/// Fields are `Some` when the prover fetches them from a remote source (Atlantic).
/// Fields are `None` when artifacts were already stored locally by the handler (SHARP / Mock).
#[derive(Default)]
pub struct AggregationArtifacts {
    pub cairo_pie: Option<Vec<u8>>,
    pub da_segment: Option<Vec<u8>>,
    pub proof: Option<Vec<u8>>,
}

pub enum Task {
    /// Submit a child CairoPIE for proving.
    CreateJob(CreateJobInfo),
    /// Create a new bucket (Atlantic) or generate a local tracking ID (SHARP / Mock).
    CreateBucket,
    /// Trigger aggregation for a batch.
    ///
    /// **Atlantic**: closes the bucket and returns the bucket ID.
    RunAggregation(String),
    /// Submit a pre-built aggregator CairoPIE.
    ///
    /// **SHARP**: submitted as an applicative job with child job keys.
    /// **Mock**: used to compute the fact hash, optionally registered on L1.
    RunAggregationWithPie(ApplicativeJobInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Processing,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskType {
    /// A regular proving job (child job).
    Job,
    /// An aggregation task.
    ///
    /// For Atlantic, this polls the bucket status.
    /// For SHARP, this polls the applicative job status.
    /// For Mock, this polls the `registerFact` tx receipt (or succeeds immediately if
    /// on-chain registration is disabled).
    Aggregation,
}

#[derive(Debug, thiserror::Error)]
pub enum ProverClientError {
    #[error("Internal prover error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("Task is invalid: {0}")]
    TaskInvalid(String),
    #[error("Fact checker error: {0}")]
    FactChecker(#[from] FactCheckerError),
    #[error("Failed to encode Cairo PIE: {0}")]
    PieEncoding(String),
    #[error("Failed to convert job key to UUID: {0}")]
    InvalidJobKey(String),
    #[error("Failed to convert fact to B256: {0}")]
    FailedToConvertFact(String),
    #[error("Failed to write file: {0}")]
    FailedToCreateTempFile(String),
    #[error("Failed to write file: {0}")]
    FailedToWriteFile(String),
    #[error("Failed to get aggregator id for bucket ID: {0}")]
    FailedToGetAggregatorId(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Invalid proof format: {0}")]
    InvalidProofFormat(String),
    #[error("Missing Cairo verifier program hash")]
    MissingCairoVerifierProgramHash,
}
