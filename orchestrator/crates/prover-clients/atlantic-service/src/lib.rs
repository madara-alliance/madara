pub mod client;
pub mod constants;
pub mod error;
pub mod types;

pub use crate::types::AtlanticQueryStatus;
use alloy::primitives::B256;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_gps_fact_checker::FactChecker;
use orchestrator_prover_client_interface::{ProverClient, ProverClientError, Task, TaskStatus};
use std::str::FromStr;
use swiftness_proof_parser::{parse, StarkProof};
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
    pub cairo_verifier_program_hash: Option<String>,
}

/// Atlantic is a SHARP wrapper service hosted by Herodotus.
pub struct AtlanticProverService {
    pub atlantic_client: AtlanticClient,
    pub fact_checker: Option<FactChecker>,
    pub atlantic_api_key: String,
    pub atlantic_network: String,
    pub cairo_verifier_program_hash: Option<String>,
}

#[async_trait]
impl ProverClient for AtlanticProverService {
    #[tracing::instrument(skip(self, task))]
    async fn submit_task(
        &self,
        task: Task,
        proof_layout: LayoutName,
        n_steps: Option<usize>,
    ) -> Result<String, ProverClientError> {
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
                    .write_zip_file(pie_file_path, true)
                    .map_err(|e| ProverClientError::FailedToWriteFile(e.to_string()))?;

                let atlantic_job_response = self
                    .atlantic_client
                    .add_job(
                        pie_file_path,
                        proof_layout,
                        self.atlantic_api_key.clone(),
                        n_steps,
                        self.atlantic_network.clone(),
                    )
                    .await?;

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
                match &self.fact_checker {
                    None => {
                        tracing::debug!("There is no Fact check registered");
                        Ok(TaskStatus::Succeeded)
                    }
                    Some(fact_checker) => {
                        tracing::debug!("Fact check registered");
                        // Cross-verification is enabled
                        let fact_str = match fact {
                            Some(f) => f,
                            None => {
                                return Ok(TaskStatus::Failed(
                                    "Cross verification enabled but no fact provided".to_string(),
                                ));
                            }
                        };

                        let fact = B256::from_str(&fact_str)
                            .map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;

                        tracing::debug!(fact = %hex::encode(fact), "Cross-verifying fact on chain");

                        if fact_checker.is_valid(&fact).await? {
                            Ok(TaskStatus::Succeeded)
                        } else {
                            Ok(TaskStatus::Failed(format!("Fact {} is not valid or not registered", hex::encode(fact))))
                        }
                    }
                }
            }

            AtlanticQueryStatus::Failed => {
                Ok(TaskStatus::Failed("Task failed while processing on Atlantic side".to_string()))
            }
        }
    }
    async fn get_proof(&self, task_id: &str) -> Result<String, ProverClientError> {
        let proof = self.atlantic_client.get_proof_by_task_id(task_id).await?;

        // Verify if it's a valid proof format
        let _: StarkProof = parse(proof.clone()).map_err(|e| ProverClientError::InvalidProofFormat(e.to_string()))?;
        Ok(proof)
    }

    /// Submit a L2 query to the Atlantic service
    ///
    /// # Arguments
    /// * `task_id` - The task id of the proof to submit
    /// * `proof` - The proof to submit
    /// * `n_steps` - The number of steps to submit
    ///
    async fn submit_l2_query(
        &self,
        task_id: &str,
        proof: &str,
        n_steps: Option<usize>
    ) -> Result<String, ProverClientError> {
        tracing::info!(
            task_id = %task_id,
            log_type = "starting",
            category = "submit_l2_query",
            function_type = "proof",
            "Submitting L2 query."
        );
        let verifier_program_hash = self.cairo_verifier_program_hash.as_ref().ok_or_else(|| {
            ProverClientError::MissingCairoVerifierProgramHash
        })?;

        let atlantic_job_response = self
            .atlantic_client
            .submit_l2_query(proof, verifier_program_hash, n_steps, &self.atlantic_network, &self.atlantic_api_key)
            .await?;

        tracing::info!(
            log_type = "completed",
            category = "submit_l2_query",
            function_type = "proof",
            "L2 query submitted."
        );

        Ok(atlantic_job_response.atlantic_query_id)
    }
}

impl AtlanticProverService {
    pub fn new(
        atlantic_client: AtlanticClient,
        atlantic_api_key: String,
        atlantic_network: String,
        fact_checker: Option<FactChecker>,
        cairo_verifier_program_hash: Option<String>,
    ) -> Self {
        Self { atlantic_client, fact_checker, atlantic_api_key, atlantic_network , cairo_verifier_program_hash }
    }

    /// Creates a new instance of `AtlanticProverService` with the given parameters.
    /// Note: If the mock fact hash is set to "true", the fact checker will be None.
    /// And the Fact check will not be performed.
    /// # Arguments
    /// * `atlantic_params` - The parameters for the Atlantic service.
    /// * `proof_layout` - The layout name for the proof.
    ///
    /// # Returns
    /// * `AtlanticProverService` - A new instance of the service.
    pub fn new_with_args(atlantic_params: &AtlanticValidatedArgs) -> Self {
        let atlantic_client =
            AtlanticClient::new_with_args(atlantic_params.atlantic_service_url.clone(), atlantic_params);

        let fact_checker = if atlantic_params.atlantic_mock_fact_hash.eq("true") {
            None
        } else {
            Some(FactChecker::new(
                atlantic_params.atlantic_rpc_node_url.clone(),
                atlantic_params.atlantic_verifier_contract_address.clone(),
                atlantic_params.atlantic_settlement_layer.clone(),
            ))
        };

        Self::new(
            atlantic_client,
            atlantic_params.atlantic_api_key.clone(),
            atlantic_params.atlantic_network.clone(),
            fact_checker,
            atlantic_params.cairo_verifier_program_hash.clone(),
        )
    }

    pub fn with_test_params(port: u16, atlantic_params: &AtlanticValidatedArgs) -> Self {
        let atlantic_client =
            AtlanticClient::new_with_args(format!("http://127.0.0.1:{}", port).parse().unwrap(), atlantic_params);
        let fact_checker = if atlantic_params.atlantic_mock_fact_hash.eq("true") {
            None
        } else {
            Some(FactChecker::new(
                atlantic_params.atlantic_rpc_node_url.clone(),
                atlantic_params.atlantic_verifier_contract_address.clone(),
                atlantic_params.atlantic_settlement_layer.clone(),
            ))
        };
        Self::new(atlantic_client, "random_api_key".to_string(), "TESTNET".to_string(), fact_checker, None)
    }
}
