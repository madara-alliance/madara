pub mod client;
mod constants;
pub mod error;
pub mod types;

use std::str::FromStr;

pub use crate::types::AtlanticQueryStatus;
use alloy::primitives::B256;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_gps_fact_checker::FactChecker;
use orchestrator_prover_client_interface::{
    AtlanticStatusType, ProverClient, ProverClientError, Task, TaskStatus, TaskType,
};
use tempfile::NamedTempFile;
use url::Url;

use crate::client::AtlanticClient;
use crate::constants::ATLANTIC_FETCH_ARTIFACTS_BASE_URL;
use crate::types::{AtlanticBucketStatus, AtlanticCairoVm, AtlanticQueryStep};

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
    pub atlantic_cairo_vm: AtlanticCairoVm,
    pub atlantic_result: AtlanticQueryStep,
}

/// Atlantic is a SHARP wrapper service hosted by Herodotus.
pub struct AtlanticProverService {
    pub atlantic_client: AtlanticClient,
    pub fact_checker: Option<FactChecker>,
    pub atlantic_api_key: String,
    pub proof_layout: LayoutName,
    pub atlantic_network: String,
    pub cairo_vm: AtlanticCairoVm,
    pub result: AtlanticQueryStep,
}

#[async_trait]
impl ProverClient for AtlanticProverService {
    #[tracing::instrument(skip(self, task))]
    async fn submit_task(&self, task: Task) -> Result<String, ProverClientError> {
        tracing::info!(
            log_type = "starting",
            category = "submit_task",
            function_type = "cairo_pie",
            "Submitting Cairo PIE task."
        );
        match task {
            Task::CreateJob(cairo_pie, bucket_id, bucket_job_index, n_steps) => {
                let temp_file =
                    NamedTempFile::new().map_err(|e| ProverClientError::FailedToCreateTempFile(e.to_string()))?;
                let pie_file_path = temp_file.path();
                cairo_pie
                    .write_zip_file(pie_file_path, true)
                    .map_err(|e| ProverClientError::FailedToWriteFile(e.to_string()))?;

                // sleep for 2 seconds to make sure the job is submitted
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                let atlantic_job_response = self
                    .atlantic_client
                    .add_job(
                        pie_file_path,
                        self.proof_layout,
                        self.cairo_vm.clone(),
                        self.result.clone(),
                        self.atlantic_api_key.clone(),
                        n_steps,
                        self.atlantic_network.clone(),
                        bucket_id,
                        bucket_job_index,
                    )
                    .await?;
                // sleep for 10 seconds to make sure the job is submitted
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                tracing::debug!("Successfully submitted task to atlantic: {:?}", atlantic_job_response);
                // The temporary file will be automatically deleted when `temp_file` goes out of scope
                Ok(atlantic_job_response.atlantic_query_id)
            }
            Task::CreateBucket => {
                let response = self.atlantic_client.create_bucket(self.atlantic_api_key.clone()).await?;
                tracing::debug!(bucket_id = %response.atlantic_bucket.id, "Successfully submitted create bucket task to atlantic: {:?}", response);
                Ok(response.atlantic_bucket.id)
            }
            Task::CloseBucket(bucket_id) => {
                let response = self.atlantic_client.close_bucket(&bucket_id, self.atlantic_api_key.clone()).await?;
                // sleep for 10 seconds to make sure that the bucket is closed
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                tracing::debug!(bucker_id = %response.atlantic_bucket.id, "Successfully submitted close bucket task to atlantic: {:?}", response);
                Ok(response.atlantic_bucket.id)
            }
        }
    }

    #[tracing::instrument(skip(self))]
    async fn get_task_status(
        &self,
        task: AtlanticStatusType,
        job_key: &str,
        _: Option<String>,
        _: bool,
    ) -> Result<TaskStatus, ProverClientError> {
        match task {
            AtlanticStatusType::Job => {
                match self.atlantic_client.get_job_status(job_key).await?.atlantic_query.status {
                    AtlanticQueryStatus::Received => Ok(TaskStatus::Processing),
                    AtlanticQueryStatus::InProgress => Ok(TaskStatus::Processing),
                    AtlanticQueryStatus::Done => Ok(TaskStatus::Succeeded),
                    AtlanticQueryStatus::Failed => {
                        Ok(TaskStatus::Failed("Task failed while processing on Atlantic side".to_string()))
                    }
                }
            }
            AtlanticStatusType::Bucket => match self.atlantic_client.get_bucket(job_key).await?.bucket.status {
                AtlanticBucketStatus::Open => Ok(TaskStatus::Processing),
                AtlanticBucketStatus::InProgress => Ok(TaskStatus::Processing),
                AtlanticBucketStatus::Done => Ok(TaskStatus::Succeeded),
                AtlanticBucketStatus::Failed => {
                    Ok(TaskStatus::Failed("Task failed while processing on Atlantic side".to_string()))
                }
            },
        }
    }

    async fn get_aggregator_task_id(
        &self,
        bucket_id: &str,
        aggregator_index: u64,
    ) -> Result<String, ProverClientError> {
        let bucket = self.atlantic_client.get_bucket(bucket_id).await?;

        Ok(bucket
            .queries
            .iter()
            .find(|query| {
                return match query.bucket_job_index {
                    Some(index) => index == aggregator_index,
                    None => false,
                };
            })
            .ok_or(ProverClientError::FailedToGetAggregatorId(bucket_id.to_string()))?
            .id
            .clone())
    }

    async fn get_task_artifacts(
        &self,
        task_id: &str,
        task_type: TaskType,
        file_name: &str,
    ) -> Result<Vec<u8>, ProverClientError> {
        match task_type {
            TaskType::Query => Ok(self
                .atlantic_client
                .get_artifacts(format!("{}/queries/{}/{}", ATLANTIC_FETCH_ARTIFACTS_BASE_URL, task_id, file_name))
                .await?),
            TaskType::Bucket => Ok(self
                .atlantic_client
                .get_artifacts(format!("{}/queries/{}/{}", ATLANTIC_FETCH_ARTIFACTS_BASE_URL, task_id, file_name))
                .await?),
        }
    }
}

impl AtlanticProverService {
    pub fn new(
        atlantic_client: AtlanticClient,
        atlantic_api_key: String,
        proof_layout: &LayoutName,
        cairo_vm: AtlanticCairoVm,
        result: AtlanticQueryStep,
        atlantic_network: String,
        fact_checker: Option<FactChecker>,
    ) -> Self {
        Self {
            atlantic_client,
            fact_checker,
            atlantic_api_key,
            proof_layout: proof_layout.to_owned(),
            cairo_vm,
            atlantic_network,
            result,
        }
    }

    /// Creates a new instance of `AtlanticProverService` with the given parameters.
    /// Note: If the mock fact hash is set to "true", the fact-checker will be None.
    /// And the Fact check will not be performed.
    /// # Arguments
    /// * `atlantic_params` - The parameters for the Atlantic service.
    /// * `proof_layout` - The layout name for the proof.
    ///
    /// # Returns
    /// * `AtlanticProverService` - A new instance of the service.
    pub fn new_with_args(atlantic_params: &AtlanticValidatedArgs, proof_layout: &LayoutName) -> Self {
        let atlantic_client =
            AtlanticClient::new_with_args(atlantic_params.atlantic_service_url.clone(), atlantic_params);

        let fact_checker = Self::get_fact_checker(atlantic_params);

        Self::new(
            atlantic_client,
            atlantic_params.atlantic_api_key.clone(),
            proof_layout,
            atlantic_params.atlantic_cairo_vm.clone(),
            atlantic_params.atlantic_result.clone(),
            atlantic_params.atlantic_network.clone(),
            fact_checker,
        )
    }

    pub fn with_test_params(port: u16, atlantic_params: &AtlanticValidatedArgs, proof_layout: &LayoutName) -> Self {
        let atlantic_client =
            AtlanticClient::new_with_args(format!("http://127.0.0.1:{}", port).parse().unwrap(), atlantic_params);

        let fact_checker = Self::get_fact_checker(atlantic_params);

        Self::new(
            atlantic_client,
            "random_api_key".to_string(),
            proof_layout,
            AtlanticCairoVm::Rust,
            AtlanticQueryStep::ProofVerificationOnL1,
            "TESTNET".to_string(),
            fact_checker,
        )
    }

    fn get_fact_checker(atlantic_params: &AtlanticValidatedArgs) -> Option<FactChecker> {
        if atlantic_params.atlantic_mock_fact_hash.eq("true") {
            None
        } else {
            Some(FactChecker::new(
                atlantic_params.atlantic_rpc_node_url.clone(),
                atlantic_params.atlantic_verifier_contract_address.clone(),
            ))
        }
    }
}
