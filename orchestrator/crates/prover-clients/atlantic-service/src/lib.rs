pub mod client;
pub mod error;
pub mod types;
use std::str::FromStr;

pub use crate::types::AtlanticQueryStatus;
use alloy::primitives::B256;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_gps_fact_checker::FactChecker;
use orchestrator_prover_client_interface::{ProverClient, ProverClientError, Task, TaskStatus};
use tempfile::NamedTempFile;
use url::Url;

use crate::client::AtlanticClient;

#[derive(Debug, Clone)]
pub struct AtlanticValidatedArgs {
    pub atlantic_api_key: String,
    pub atlantic_service_url: Url,
    pub atlantic_rpc_node_url: Url,
    pub atlantic_verifier_contract_address: String,
    pub atlantic_settlement_layer: String,
    pub atlantic_mock_fact_hash: String,
    pub atlantic_prover_type: String,
    pub atlantic_network: String,
}

/// Atlantic is a SHARP wrapper service hosted by Herodotus.
pub struct AtlanticProverService {
    pub atlantic_client: AtlanticClient,
    pub fact_checker: FactChecker,
    pub atlantic_api_key: String,
    pub proof_layout: LayoutName,
    pub atlantic_network: String,
}

#[async_trait]
impl ProverClient for AtlanticProverService {
    #[tracing::instrument(skip(self, task))]
    async fn submit_task(&self, task: Task, n_steps: Option<usize>) -> Result<String, ProverClientError> {
        tracing::info!(
            log_type = "starting",
            category = "submit_task",
            function_type = "cairo_pie",
            "Submitting Cairo PIE task."
        );
        match task {
            Task::CairoPie(cairo_pie) => {
                let temp_file =
                    NamedTempFile::new().map_err(|e| ProverClientError::FailedToCreateTempFile(e.to_string()))?;
                let pie_file_path = temp_file.path();
                cairo_pie
                    .write_zip_file(pie_file_path)
                    .map_err(|e| ProverClientError::FailedToWriteFile(e.to_string()))?;

                // sleep for 2 seconds to make sure the job is submitted
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                let atlantic_job_response = self
                    .atlantic_client
                    .add_job(
                        pie_file_path,
                        self.proof_layout,
                        self.atlantic_api_key.clone(),
                        n_steps,
                        self.atlantic_network.clone(),
                    )
                    .await?;
                // sleep for 10 seconds to make sure the job is submitted
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                tracing::debug!("Successfully submitted task to atlantic: {:?}", atlantic_job_response);
                // The temporary file will be automatically deleted when `temp_file` goes out of scope
                Ok(atlantic_job_response.atlantic_query_id)
            }
        }
    }

    #[tracing::instrument(skip(self))]
    async fn get_task_status(
        &self,
        job_key: &str,
        fact: Option<String>,
        cross_verify: bool,
    ) -> Result<TaskStatus, ProverClientError> {
        let res = self.atlantic_client.get_job_status(job_key).await?;

        match res.atlantic_query.status {
            AtlanticQueryStatus::Received => Ok(TaskStatus::Processing),
            AtlanticQueryStatus::InProgress => Ok(TaskStatus::Processing),

            AtlanticQueryStatus::Done => {
                if !cross_verify {
                    tracing::debug!("Skipping cross-verification as it's disabled");
                    return Ok(TaskStatus::Succeeded);
                }

                // Cross verification is enabled
                let fact_str = match fact {
                    Some(f) => f,
                    None => {
                        return Ok(TaskStatus::Failed("Cross verification enabled but no fact provided".to_string()));
                    }
                };

                let fact =
                    B256::from_str(&fact_str).map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;

                tracing::debug!(fact = %hex::encode(fact), "Cross-verifying fact on chain");

                if self.fact_checker.is_valid(&fact).await? {
                    Ok(TaskStatus::Succeeded)
                } else {
                    Ok(TaskStatus::Failed(format!("Fact {} is not valid or not registered", hex::encode(fact))))
                }
            }

            AtlanticQueryStatus::Failed => {
                Ok(TaskStatus::Failed("Task failed while processing on Atlantic side".to_string()))
            }
        }
    }
}

impl AtlanticProverService {
    pub fn new(
        atlantic_client: AtlanticClient,
        fact_checker: FactChecker,
        atlantic_api_key: String,
        proof_layout: &LayoutName,
        atlantic_network: String,
    ) -> Self {
        Self {
            atlantic_client,
            fact_checker,
            atlantic_api_key,
            proof_layout: proof_layout.to_owned(),
            atlantic_network,
        }
    }

    pub fn new_with_args(atlantic_params: &AtlanticValidatedArgs, proof_layout: &LayoutName) -> Self {
        let atlantic_client =
            AtlanticClient::new_with_args(atlantic_params.atlantic_service_url.clone(), atlantic_params);

        let fact_checker = FactChecker::new(
            atlantic_params.atlantic_rpc_node_url.clone(),
            atlantic_params.atlantic_verifier_contract_address.clone(),
        );

        Self::new(
            atlantic_client,
            fact_checker,
            atlantic_params.atlantic_api_key.clone(),
            proof_layout,
            atlantic_params.atlantic_network.clone(),
        )
    }

    pub fn with_test_params(port: u16, atlantic_params: &AtlanticValidatedArgs, proof_layout: &LayoutName) -> Self {
        let atlantic_client =
            AtlanticClient::new_with_args(format!("http://127.0.0.1:{}", port).parse().unwrap(), atlantic_params);
        let fact_checker = FactChecker::new(
            atlantic_params.atlantic_rpc_node_url.clone(),
            atlantic_params.atlantic_verifier_contract_address.clone(),
        );
        Self::new(atlantic_client, fact_checker, "random_api_key".to_string(), proof_layout, "TESTNET".to_string())
    }
}
