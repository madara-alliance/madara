pub mod client;
mod constants;
pub mod error;
pub mod types;

use std::str::FromStr;

use crate::types::CairoJobStatus;
use alloy::primitives::B256;
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine;
use orchestrator_gps_fact_checker::FactChecker;
use orchestrator_prover_client_interface::{
    ApplicativeJobInfo, CreateJobInfo, ProverClient, ProverClientError, Task, TaskStatus, TaskType,
};
use tempfile::NamedTempFile;
use url::Url;
use uuid::Uuid;

use crate::client::SharpClient;

pub const SHARP_SETTINGS_NAME: &str = "sharp";

#[derive(Debug, Clone)]
pub struct SharpValidatedArgs {
    pub sharp_customer_id: String,
    pub sharp_url: Url,
    pub sharp_user_crt: String,
    pub sharp_user_key: String,
    pub sharp_rpc_node_url: Url,
    pub sharp_server_crt: String,
    pub gps_verifier_contract_address: String,
    pub sharp_settlement_layer: String,
    pub sharp_offchain_proof: bool,
}

/// SHARP (aka GPS) is a shared proving service hosted by Starkware.
pub struct SharpProverService {
    sharp_client: SharpClient,
    fact_checker: FactChecker,
}

/// Encode a CairoPIE as base64 for the SHARP API.
///
/// Writes the CairoPIE to a temporary zip file, reads the bytes,
/// and returns the base64-encoded string.
fn encode_cairo_pie_base64(
    cairo_pie: &cairo_vm::vm::runners::cairo_pie::CairoPie,
) -> Result<String, ProverClientError> {
    let temp_file = NamedTempFile::new().map_err(|e| ProverClientError::FailedToCreateTempFile(e.to_string()))?;
    cairo_pie
        .write_zip_file(temp_file.path(), true)
        .map_err(|e| ProverClientError::FailedToWriteFile(e.to_string()))?;
    let zip_bytes = std::fs::read(temp_file.path()).map_err(|e| ProverClientError::PieEncoding(e.to_string()))?;
    Ok(general_purpose::STANDARD.encode(&zip_bytes))
}

#[async_trait]
impl ProverClient for SharpProverService {
    #[tracing::instrument(skip(self, task), ret, err)]
    async fn submit_task(&self, task: Task) -> Result<String, ProverClientError> {
        match task {
            Task::CreateJob(CreateJobInfo { cairo_pie, .. }) => {
                tracing::info!(
                    log_type = "starting",
                    category = "submit_task",
                    function_type = "cairo_pie",
                    "Submitting Cairo PIE to SHARP."
                );
                let encoded_pie = encode_cairo_pie_base64(&cairo_pie)?;
                let cairo_job_key = Uuid::new_v4().to_string();

                self.sharp_client.add_job(&encoded_pie, &cairo_job_key).await?;

                tracing::info!(
                    log_type = "completed",
                    category = "submit_task",
                    function_type = "cairo_pie",
                    cairo_job_key = %cairo_job_key,
                    "Cairo PIE submitted to SHARP."
                );
                Ok(cairo_job_key)
            }
            Task::CreateBucket => {
                // SHARP has no bucket concept. Return a local UUID for tracking.
                let bucket_id = Uuid::new_v4().to_string();
                tracing::debug!(bucket_id = %bucket_id, "Generated local bucket ID (SHARP has no remote buckets)");
                Ok(bucket_id)
            }
            Task::CloseBucket(bucket_id) => {
                // No-op for SHARP. The real aggregation work happens in the aggregator job handler.
                tracing::debug!(bucket_id = %bucket_id, "CloseBucket is a no-op for SHARP");
                Ok(bucket_id)
            }
            Task::SubmitApplicativeJob(ApplicativeJobInfo { cairo_pie, children_cairo_job_keys }) => {
                tracing::info!(
                    log_type = "starting",
                    category = "submit_task",
                    function_type = "applicative_job",
                    num_children = children_cairo_job_keys.len(),
                    "Submitting applicative job to SHARP."
                );
                let encoded_pie = encode_cairo_pie_base64(&cairo_pie)?;
                let cairo_job_key = Uuid::new_v4().to_string();

                self.sharp_client.add_applicative_job(&encoded_pie, &cairo_job_key, &children_cairo_job_keys).await?;

                tracing::info!(
                    log_type = "completed",
                    category = "submit_task",
                    function_type = "applicative_job",
                    cairo_job_key = %cairo_job_key,
                    "Applicative job submitted to SHARP."
                );
                Ok(cairo_job_key)
            }
        }
    }

    #[tracing::instrument(skip(self), ret, err)]
    async fn get_task_status(
        &self,
        task: TaskType,
        job_key: &str,
        fact: Option<String>,
        cross_verify: bool,
    ) -> Result<TaskStatus, ProverClientError> {
        match task {
            TaskType::Job => {
                // For child jobs: Succeeded when validated (PROCESSED, or IN_PROGRESS + validation_done).
                // This allows the ProofCreation job to complete before the proof is fully on-chain,
                // since on-chain registration happens at the applicative job level.
                let res = self.sharp_client.get_job_status(job_key).await?;

                match res.status {
                    CairoJobStatus::Failed => {
                        tracing::error!(cairo_job_key = %job_key, "SHARP child job FAILED");
                        Ok(TaskStatus::Failed(res.error_log.unwrap_or_default()))
                    }
                    CairoJobStatus::Invalid => {
                        tracing::warn!(cairo_job_key = %job_key, "SHARP child job INVALID");
                        Ok(TaskStatus::Failed(format!("Job is invalid: {:?}", res.invalid_reason.unwrap_or_default())))
                    }
                    CairoJobStatus::Unknown => {
                        tracing::warn!(cairo_job_key = %job_key, "SHARP child job UNKNOWN");
                        Ok(TaskStatus::Failed(format!("Job not found: {}", job_key)))
                    }
                    CairoJobStatus::Processed | CairoJobStatus::Onchain => {
                        tracing::info!(cairo_job_key = %job_key, status = ?res.status, "SHARP child job validated");
                        Ok(TaskStatus::Succeeded)
                    }
                    CairoJobStatus::InProgress => {
                        if res.validation_done == Some(true) {
                            tracing::info!(cairo_job_key = %job_key, "SHARP child job validated (IN_PROGRESS + validation_done)");
                            Ok(TaskStatus::Succeeded)
                        } else {
                            tracing::debug!(cairo_job_key = %job_key, "SHARP child job still in progress");
                            Ok(TaskStatus::Processing)
                        }
                    }
                    CairoJobStatus::NotCreated => {
                        tracing::debug!(cairo_job_key = %job_key, "SHARP child job not yet created");
                        Ok(TaskStatus::Processing)
                    }
                }
            }
            TaskType::ApplicativeJob => {
                // For applicative jobs: Succeeded only when ONCHAIN (fact registered).
                let res = self.sharp_client.get_job_status(job_key).await?;

                match res.status {
                    CairoJobStatus::Onchain => {
                        if cross_verify {
                            if let Some(fact_str) = fact {
                                let fact = B256::from_str(&fact_str)
                                    .map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;
                                if self.fact_checker.is_valid(&fact).await? {
                                    tracing::info!(cairo_job_key = %job_key, "Applicative job ONCHAIN, fact verified");
                                    Ok(TaskStatus::Succeeded)
                                } else {
                                    tracing::error!(cairo_job_key = %job_key, "Applicative job ONCHAIN but fact invalid");
                                    Ok(TaskStatus::Failed(format!(
                                        "Fact {} is not valid or not registered",
                                        hex::encode(fact)
                                    )))
                                }
                            } else {
                                tracing::debug!(
                                    cairo_job_key = %job_key,
                                    "No fact provided for cross-verification, considering successful"
                                );
                                Ok(TaskStatus::Succeeded)
                            }
                        } else {
                            tracing::info!(cairo_job_key = %job_key, "Applicative job ONCHAIN");
                            Ok(TaskStatus::Succeeded)
                        }
                    }
                    CairoJobStatus::Failed => {
                        tracing::error!(cairo_job_key = %job_key, "Applicative job FAILED");
                        Ok(TaskStatus::Failed(res.error_log.unwrap_or_default()))
                    }
                    CairoJobStatus::Invalid => {
                        tracing::warn!(cairo_job_key = %job_key, "Applicative job INVALID");
                        Ok(TaskStatus::Failed(format!(
                            "Applicative job is invalid: {:?}",
                            res.invalid_reason.unwrap_or_default()
                        )))
                    }
                    CairoJobStatus::Unknown => {
                        tracing::warn!(cairo_job_key = %job_key, "Applicative job UNKNOWN");
                        Ok(TaskStatus::Failed(format!("Applicative job not found: {}", job_key)))
                    }
                    _ => {
                        tracing::debug!(cairo_job_key = %job_key, status = ?res.status, "Applicative job still processing");
                        Ok(TaskStatus::Processing)
                    }
                }
            }
            TaskType::Bucket => {
                // SHARP has no remote buckets. Return Processing as a safe default.
                tracing::debug!("Bucket status check is a no-op for SHARP");
                Ok(TaskStatus::Processing)
            }
        }
    }

    async fn get_proof(&self, task_id: &str) -> Result<String, ProverClientError> {
        Ok(self.sharp_client.get_proof(task_id).await?)
    }

    async fn submit_l2_query(
        &self,
        _task_id: &str,
        _fact: &str,
        _n_steps: Option<usize>,
    ) -> Result<String, ProverClientError> {
        Err(ProverClientError::Internal(Box::new(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "submit_l2_query is not supported by SHARP for L2",
        ))))
    }

    async fn get_task_artifacts(&self, _task_id: &str, _file_name: &str) -> Result<Vec<u8>, ProverClientError> {
        Err(ProverClientError::Internal(Box::new(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SHARP does not support remote artifact fetching; artifacts are produced locally",
        ))))
    }

    async fn get_aggregator_task_id(&self, _bucket_id: &str) -> Result<String, ProverClientError> {
        Err(ProverClientError::Internal(Box::new(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SHARP does not use remote aggregator task IDs; aggregation is done locally",
        ))))
    }
}

impl SharpProverService {
    pub fn new(sharp_client: SharpClient, fact_checker: FactChecker) -> Self {
        Self { sharp_client, fact_checker }
    }

    pub fn new_with_args(sharp_params: &SharpValidatedArgs) -> Self {
        let sharp_client = SharpClient::new_with_args(sharp_params.sharp_url.clone(), sharp_params);
        let fact_checker = FactChecker::new(
            sharp_params.sharp_rpc_node_url.clone(),
            sharp_params.gps_verifier_contract_address.clone(),
            sharp_params.sharp_settlement_layer.clone(),
        );
        Self::new(sharp_client, fact_checker)
    }

    pub fn with_test_params(port: u16, sharp_params: &SharpValidatedArgs) -> Self {
        let sharp_client = SharpClient::new_with_args(
            format!("http://127.0.0.1:{}", port).parse().expect("Failed to create sharp client with the given params"),
            sharp_params,
        );
        let fact_checker = FactChecker::new(
            sharp_params.sharp_rpc_node_url.clone(),
            sharp_params.gps_verifier_contract_address.clone(),
            sharp_params.sharp_settlement_layer.clone(),
        );
        Self::new(sharp_client, fact_checker)
    }
}
