pub mod client;
mod constants;
pub mod error;
pub mod metrics;
pub mod types;

use std::str::FromStr;

use crate::types::{CairoJobStatus, SharpGetStatusResponse};
use alloy::primitives::B256;
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine;
use orchestrator_gps_fact_checker::FactChecker;
use orchestrator_prover_client_interface::{
    AggregationArtifacts, ApplicativeJobInfo, CreateJobInfo, ProverClient, ProverClientError, Task, TaskStatus,
    TaskType,
};
use tempfile::NamedTempFile;
use url::Url;

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
}

/// SHARP (aka GPS) is a shared proving service hosted by Starkware.
pub struct SharpProverService {
    sharp_client: SharpClient,
    fact_checker: FactChecker,
}

/// Encode a [`CairoPie`] struct via the temp-file dance.
/// Used for individual child jobs where we receive a PIE by value.
fn encode_cairo_pie_from_struct(
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
            Task::CreateJob(CreateJobInfo { cairo_pie, dedup_id, .. }) => {
                tracing::info!(
                    log_type = "starting",
                    category = "submit_task",
                    function_type = "cairo_pie",
                    "Submitting Cairo PIE to SHARP."
                );
                // Use the orchestrator's dedup_id as the cairo_job_key so retries
                // hit the same SHARP server-side job instead of creating duplicates.
                let cairo_job_key = dedup_id;

                // Idempotency: if SHARP already knows this key and it hasn't failed,
                // skip re-submission.
                if let Some(existing) = self.check_existing_job(&cairo_job_key).await? {
                    metrics::SHARP_METRICS.record_idempotency_hit("cairo_pie");
                    return Ok(existing);
                }

                let encoded_pie = encode_cairo_pie_from_struct(&cairo_pie)?;
                self.sharp_client.add_job(&encoded_pie, &cairo_job_key).await?;
                tracing::info!(
                    log_type = "completed",
                    category = "submit_task",
                    cairo_job_key = %cairo_job_key,
                    "Cairo PIE submitted to SHARP."
                );
                Ok(cairo_job_key)
            }
            Task::CreateBucket => {
                let bucket_id = uuid::Uuid::new_v4().to_string();
                tracing::debug!(bucket_id = %bucket_id, "Generated local bucket ID (SHARP has no remote buckets)");
                Ok(bucket_id)
            }
            Task::RunAggregation(_) => Err(ProverClientError::TaskInvalid(
                "SHARP does not support bucket-based aggregation. Use RunAggregationWithPie.".to_string(),
            )),
            Task::RunAggregationWithPie(ApplicativeJobInfo {
                cairo_pie_zip_bytes,
                children_cairo_job_keys,
                fact_hash,
            }) => {
                // Use the fact hash (hex) as the cairo_job_key — deterministic for the
                // same aggregation, and recommended by the SHARP team as the natural
                // idempotency key for applicative jobs.
                let cairo_job_key = match fact_hash {
                    Some(fact) => format!("0x{}", hex::encode(fact)),
                    None => {
                        return Err(ProverClientError::TaskInvalid(
                            "fact_hash is required for SHARP applicative jobs".to_string(),
                        ))
                    }
                };
                tracing::info!(
                    log_type = "starting",
                    category = "submit_task",
                    function_type = "applicative_job",
                    num_children = children_cairo_job_keys.len(),
                    cairo_job_key = %cairo_job_key,
                    "Submitting applicative job to SHARP."
                );

                // Idempotency: skip if already submitted.
                if let Some(existing) = self.check_existing_job(&cairo_job_key).await? {
                    metrics::SHARP_METRICS.record_idempotency_hit("applicative_job");
                    return Ok(existing);
                }

                let encoded_pie = general_purpose::STANDARD.encode(&cairo_pie_zip_bytes);
                self.sharp_client.add_applicative_job(&encoded_pie, &cairo_job_key, &children_cairo_job_keys).await?;
                tracing::info!(
                    log_type = "completed",
                    category = "submit_task",
                    cairo_job_key = %cairo_job_key,
                    "Applicative job submitted to SHARP."
                );
                Ok(cairo_job_key)
            }
        }
    }

    #[tracing::instrument(skip(self, fact), ret, err)]
    async fn get_task_status(
        &self,
        task: TaskType,
        job_key: &str,
        fact: Option<String>,
    ) -> Result<TaskStatus, ProverClientError> {
        let res = self.sharp_client.get_job_status(job_key).await?;
        let task_label = match task {
            TaskType::Job => "job",
            TaskType::Aggregation => "aggregation",
        };
        metrics::SHARP_METRICS.record_job_status(task_label, &format!("{:?}", res.status));

        // For aggregation + Processed: cross-verify the fact on-chain before
        // calling the pure mapper. This is the only async / side-effecting step.
        let fact_verified_on_chain = if task == TaskType::Aggregation && res.status == CairoJobStatus::Processed {
            if let Some(fact_str) = &fact {
                let fact =
                    B256::from_str(fact_str).map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;
                Some(self.fact_checker.is_valid(&fact).await?)
            } else {
                return Err(ProverClientError::TaskInvalid(String::from(
                    "Fact required for checking aggregator job status for SHARP prover",
                )));
            }
        } else {
            None
        };

        let status = map_sharp_status(&task, job_key, &res, fact_verified_on_chain);

        match &status {
            TaskStatus::Succeeded => {
                tracing::info!(cairo_job_key = %job_key, task_type = task_label, "Job succeeded");
            }
            TaskStatus::Processing => {
                tracing::debug!(cairo_job_key = %job_key, task_type = task_label, sharp_status = ?res.status, "Job still processing");
            }
            TaskStatus::Failed(err) => {
                tracing::warn!(cairo_job_key = %job_key, task_type = task_label, error = %err, "Job failed");
            }
        }

        Ok(status)
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

    /// SHAPR does not create the aggregator PIE, so it can't return it.
    /// PIE + DA artifacts are stored by the handler during process_job.
    /// Proof can optionally be fetched if `include_proof` is true.
    async fn get_aggregation_artifacts(
        &self,
        external_id: &str,
        include_proof: bool,
    ) -> Result<AggregationArtifacts, ProverClientError> {
        let proof = if include_proof {
            let proof_json = self.get_proof(external_id).await?;
            Some(proof_json.into_bytes())
        } else {
            None
        };
        Ok(AggregationArtifacts { cairo_pie: None, da_segment: None, proof })
    }
}

/// Pure status-mapping logic: maps a SHARP `CairoJobStatus` + `TaskType` to
/// the orchestrator's `TaskStatus`.
///
/// `fact_verified_on_chain` is only relevant for `Aggregation + Processed`:
/// - `Some(true)` → fact confirmed on GPS verifier → `Succeeded`
/// - `Some(false)` → fact not yet on-chain → `Processing`
/// - `None` → no fact provided, skip cross-check → `Succeeded`
///
/// Extracted as a free function so it can be unit-tested without a live SHARP
/// client or fact checker.
pub fn map_sharp_status(
    task: &TaskType,
    job_key: &str,
    res: &SharpGetStatusResponse,
    fact_verified_on_chain: Option<bool>,
) -> TaskStatus {
    match task {
        TaskType::Job => match res.status {
            CairoJobStatus::Failed => TaskStatus::Failed(res.error_log.clone().unwrap_or_default()),
            CairoJobStatus::Invalid => {
                TaskStatus::Failed(format!("Job is invalid: {:?}", res.invalid_reason.clone().unwrap_or_default()))
            }
            CairoJobStatus::Unknown => TaskStatus::Failed(format!("Job not found: {}", job_key)),
            CairoJobStatus::Processed => TaskStatus::Succeeded,
            CairoJobStatus::InProgress => {
                if res.validation_done == Some(true) {
                    TaskStatus::Succeeded
                } else {
                    TaskStatus::Processing
                }
            }
            CairoJobStatus::NotCreated => TaskStatus::Processing,
        },
        TaskType::Aggregation => match res.status {
            CairoJobStatus::Failed => TaskStatus::Failed(res.error_log.clone().unwrap_or_default()),
            CairoJobStatus::Invalid => TaskStatus::Failed(format!(
                "Applicative job is invalid: {:?}",
                res.invalid_reason.clone().unwrap_or_default()
            )),
            CairoJobStatus::Unknown => TaskStatus::Failed(format!("Applicative job not found: {}", job_key)),
            CairoJobStatus::Processed => match fact_verified_on_chain {
                Some(true) => TaskStatus::Succeeded,
                Some(false) => TaskStatus::Processing,
                None => TaskStatus::Succeeded,
            },
            _ => TaskStatus::Processing,
        },
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

    /// Check if a job with the given key already exists on SHARP.
    ///
    /// Returns `Some(key)` to short-circuit submission if the job is already known
    /// (any state — including Failed). A Failed job will surface through
    /// `get_task_status` → `TaskStatus::Failed`, letting the operator investigate
    /// rather than silently retrying with the same inputs.
    ///
    /// Returns `None` only when SHARP reports `Unknown` (job not found).
    async fn check_existing_job(&self, cairo_job_key: &str) -> Result<Option<String>, ProverClientError> {
        let res = self.sharp_client.get_job_status(cairo_job_key).await?;
        match res.status {
            CairoJobStatus::Unknown => Ok(None),
            status => {
                tracing::info!(cairo_job_key = %cairo_job_key, status = ?status, "Job already exists on SHARP, skipping submission");
                Ok(Some(cairo_job_key.to_string()))
            }
        }
    }
}
