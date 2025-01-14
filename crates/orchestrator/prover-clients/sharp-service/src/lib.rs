pub mod client;
pub mod error;
mod types;

use std::str::FromStr;

use alloy::primitives::B256;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use gps_fact_checker::FactChecker;
use prover_client_interface::{ProverClient, ProverClientError, Task, TaskStatus};
use starknet_os::sharp::CairoJobStatus;
use uuid::Uuid;

use crate::client::SharpClient;

pub const SHARP_SETTINGS_NAME: &str = "sharp";

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
}

/// SHARP (aka GPS) is a shared proving service hosted by Starkware.
pub struct SharpProverService {
    sharp_client: SharpClient,
    fact_checker: FactChecker,
}

#[async_trait]
impl ProverClient for SharpProverService {
    #[tracing::instrument(skip(self, task), ret, err)]
    async fn submit_task(&self, task: Task, proof_layout: LayoutName) -> Result<String, ProverClientError> {
        tracing::info!(
            log_type = "starting",
            category = "submit_task",
            function_type = "cairo_pie",
            "Submitting Cairo PIE task."
        );
        match task {
            Task::CairoPie(cairo_pie) => {
                let encoded_pie =
                    starknet_os::sharp::pie::encode_pie_mem(*cairo_pie).map_err(ProverClientError::PieEncoding)?;
                let (_, job_key) = self.sharp_client.add_job(&encoded_pie, proof_layout).await?;
                tracing::info!(
                    log_type = "completed",
                    category = "submit_task",
                    function_type = "cairo_pie",
                    "Cairo PIE task submitted."
                );
                Ok(job_key.to_string())
            }
        }
    }

    #[tracing::instrument(skip(self), ret, err)]
    async fn get_task_status(&self, job_key: &str, fact: &str) -> Result<TaskStatus, ProverClientError> {
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
            CairoJobStatus::ONCHAIN => {
                let fact = B256::from_str(fact).map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;
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
        }
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
        );
        Self::new(sharp_client, fact_checker)
    }
}
