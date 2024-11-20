use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use gps_fact_checker::FactCheckerError;
use mockall::automock;

/// Prover client provides an abstraction over different proving services that do the following:
/// - Accept a task containing Cairo intermediate execution artifacts (in PIE format)
/// - Aggregate multiple tasks and prove the execution (of the bootloader program where PIEs are
///   inputs)
/// - Register the proof onchain (individiual proof facts available for each task)
///
/// A common Madara workflow would be single task per block (SNOS execution result) or per block
/// span (SNAR).
#[automock]
#[async_trait]
pub trait ProverClient: Send + Sync {
    async fn submit_task(&self, task: Task, proof_layout: LayoutName) -> Result<String, ProverClientError>;
    async fn get_task_status(&self, task_id: &str, fact: &str) -> Result<TaskStatus, ProverClientError>;
}

pub enum Task {
    CairoPie(Box<CairoPie>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Processing,
    Succeeded,
    Failed(String),
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
    PieEncoding(#[source] starknet_os::error::SnOsError),
    #[error("Failed to convert job key to UUID: {0}")]
    InvalidJobKey(String),
    #[error("Failed to convert fact to B256: {0}")]
    FailedToConvertFact(String),
    #[error("Failed to write file: {0}")]
    FailedToCreateTempFile(String),
    #[error("Failed to write file: {0}")]
    FailedToWriteFile(String),
}
