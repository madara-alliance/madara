pub mod client;
mod constants;
pub mod error;
mod types;

use std::str::FromStr;

use alloy_primitives::B256;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_gps_fact_checker::FactChecker;
use orchestrator_prover_client_interface::{
    CreateJobInfo, ProverClient, ProverClientError, Task, TaskStatus, TaskType,
};
use starknet_os::sharp::CairoJobStatus;
use uuid::Uuid;

use crate::client::SharpClient;

pub const SHARP_SETTINGS_NAME: &str = "sharp";

use crate::constants::SHARP_FETCH_ARTIFACTS_BASE_URL;
use url::Url;

#[derive(Debug, Clone)]
pub struct SharpValidatedArgs {
    pub sharp_customer_id: String,
    pub sharp_url: Url,
    pub sharp_user_crt: String,
    pub sharp_user_key: String,
    pub sharp_rpc_node_url: Url,
    pub sharp_server_crt: String,
    pub sharp_proof_layout: String,
    pub gps_verifier_contract_address: String,
    pub sharp_settlement_layer: String,
}

/// SHARP (aka GPS) is a shared proving service hosted by Starkware.
pub struct SharpProverService {
    sharp_client: SharpClient,
    fact_checker: FactChecker,
    proof_layout: LayoutName,
}

#[async_trait]
impl ProverClient for SharpProverService {
    #[tracing::instrument(skip(self, task), ret, err)]
    async fn submit_task(&self, task: Task) -> Result<String, ProverClientError> {
        tracing::info!(
            log_type = "starting",
            category = "submit_task",
            function_type = "cairo_pie",
            "Submitting Cairo PIE task."
        );
        match task {
            Task::CreateJob(CreateJobInfo { cairo_pie, .. }) => {
                let encoded_pie =
                    starknet_os::sharp::pie::encode_pie_mem(*cairo_pie).map_err(ProverClientError::PieEncoding)?;
                let (_, job_key) = self.sharp_client.add_job(&encoded_pie, self.proof_layout).await?;
                tracing::info!(
                    log_type = "completed",
                    category = "submit_task",
                    function_type = "cairo_pie",
                    "Cairo PIE task submitted."
                );
                Ok(job_key.to_string())
            }
            Task::CreateBucket => {
                let response = self.sharp_client.create_bucket().await?;
                Ok(response.bucket_id.to_string())
            }
            Task::CloseBucket(bucket_id) => {
                self.sharp_client.close_bucket(&bucket_id).await?;
                Ok(bucket_id)
            }
        }
    }

    #[tracing::instrument(skip(self), ret, err)]
    async fn get_task_status(
        &self,
        _task: TaskType,
        job_key: &str,
        fact: Option<String>,
        _cross_verify: bool,
    ) -> Result<TaskStatus, ProverClientError> {
        tracing::info!(
            log_type = "starting",
            category = "get_task_status",
            function_type = "cairo_pie",
            "Getting Cairo PIE task status."
        );
        let job_key = Uuid::from_str(job_key)
            .map_err(|e| ProverClientError::InvalidJobKey(format!("Failed to convert {} to UUID {}", job_key, e)))?;
        let res = self.sharp_client.get_job_status(&job_key).await?;

        match res.status {
            // TODO : We would need to remove the FAILED, UNKNOWN, NOT_CREATED status as it is not in the sharp client
            // response specs : https://docs.google.com/document/d/1-9ggQoYmjqAtLBGNNR2Z5eLreBmlckGYjbVl0khtpU0
            // We are waiting for the official public API spec before making changes

            // The `unwrap_or_default`s below is safe as it is just returning if any error logs
            // present
            CairoJobStatus::FAILED => {
                tracing::error!(
                    log_type = "failed",
                    category = "get_task_status",
                    function_type = "cairo_pie",
                    "Cairo PIE task status: FAILED."
                );
                Ok(TaskStatus::Failed(res.error_log.unwrap_or_default()))
            }
            CairoJobStatus::INVALID => {
                tracing::warn!(
                    log_type = "completed",
                    category = "get_task_status",
                    function_type = "cairo_pie",
                    "Cairo PIE task status: INVALID."
                );
                Ok(TaskStatus::Failed(format!("Task is invalid: {:?}", res.invalid_reason.unwrap_or_default())))
            }
            CairoJobStatus::UNKNOWN => {
                tracing::warn!(
                    log_type = "unknown",
                    category = "get_task_status",
                    function_type = "cairo_pie",
                    "Cairo PIE task status: UNKNOWN."
                );
                Ok(TaskStatus::Failed(format!("Task not found: {}", job_key)))
            }
            CairoJobStatus::IN_PROGRESS | CairoJobStatus::NOT_CREATED | CairoJobStatus::PROCESSED => {
                tracing::info!(
                    log_type = "in_progress",
                    category = "get_task_status",
                    function_type = "cairo_pie",
                    "Cairo PIE task status: IN_PROGRESS, NOT_CREATED, or PROCESSED."
                );
                Ok(TaskStatus::Processing)
            }
            CairoJobStatus::ONCHAIN => match fact {
                Some(fact_str) => {
                    let fact =
                        B256::from_str(&fact_str).map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;

                    if self.fact_checker.is_valid(&fact).await? {
                        tracing::info!(
                            log_type = "onchain",
                            category = "get_task_status",
                            function_type = "cairo_pie",
                            "Cairo PIE task status: ONCHAIN and fact is valid."
                        );
                        Ok(TaskStatus::Succeeded)
                    } else {
                        tracing::error!(
                            log_type = "onchain_failed",
                            category = "get_task_status",
                            function_type = "cairo_pie",
                            "Cairo PIE task status: ONCHAIN and fact is not valid."
                        );
                        Ok(TaskStatus::Failed(format!("Fact {} is not valid or not registered", hex::encode(fact))))
                    }
                }
                None => {
                    tracing::debug!("No fact provided for verification, considering job successful");
                    Ok(TaskStatus::Succeeded)
                }
            },
        }
    }

    /// TODO: We need to implement this function for the prover client while adding the testcase
    /// or while using the sharp prover client.
    async fn get_proof(&self, _task_id: &str) -> Result<String, ProverClientError> {
        todo!()
    }

    /// TODO: We need to implement this function for the prover client while adding the testcase
    /// or while using the sharp prover client.
    async fn submit_l2_query(
        &self,
        _task_id: &str,
        _fact: &str,
        _n_steps: Option<usize>,
    ) -> Result<String, ProverClientError> {
        todo!()
    }

    async fn get_task_artifacts(&self, task_id: &str, file_name: &str) -> Result<Vec<u8>, ProverClientError> {
        Ok(self
            .sharp_client
            .get_artifacts(format!("{}/queries/{}/{}", SHARP_FETCH_ARTIFACTS_BASE_URL, task_id, file_name))
            .await?)
    }

    async fn get_aggregator_task_id(&self, bucket_id: &str, _: u64) -> Result<String, ProverClientError> {
        Ok(self.sharp_client.get_aggregator_task_id(bucket_id).await?.task_id)
    }
}

impl SharpProverService {
    pub fn new(sharp_client: SharpClient, fact_checker: FactChecker, proof_layout: &LayoutName) -> Self {
        Self { sharp_client, fact_checker, proof_layout: proof_layout.to_owned() }
    }

    pub fn new_with_args(sharp_params: &SharpValidatedArgs, proof_layout: &LayoutName) -> Self {
        let sharp_client = SharpClient::new_with_args(sharp_params.sharp_url.clone(), sharp_params);
        let fact_checker = FactChecker::new(
            sharp_params.sharp_rpc_node_url.clone(),
            sharp_params.gps_verifier_contract_address.clone(),
            sharp_params.sharp_settlement_layer.clone(),
        );
        Self::new(sharp_client, fact_checker, proof_layout)
    }

    pub fn with_test_params(port: u16, sharp_params: &SharpValidatedArgs, proof_layout: &LayoutName) -> Self {
        let sharp_client = SharpClient::new_with_args(
            format!("http://127.0.0.1:{}", port).parse().expect("Failed to create sharp client with the given params"),
            sharp_params,
        );
        let fact_checker = FactChecker::new(
            sharp_params.sharp_rpc_node_url.clone(),
            sharp_params.gps_verifier_contract_address.clone(),
            sharp_params.sharp_settlement_layer.clone(),
        );
        Self::new(sharp_client, fact_checker, proof_layout)
    }
}
