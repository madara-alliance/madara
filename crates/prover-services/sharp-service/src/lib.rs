pub mod client;
pub mod config;
pub mod error;
mod types;

use std::str::FromStr;

use alloy::primitives::B256;
use async_trait::async_trait;
use gps_fact_checker::FactChecker;
use prover_client_interface::{ProverClient, ProverClientError, Task, TaskStatus};
use starknet_os::sharp::CairoJobStatus;
use utils::settings::Settings;
use uuid::Uuid;

use crate::client::SharpClient;
use crate::config::SharpConfig;

pub const SHARP_SETTINGS_NAME: &str = "sharp";

/// SHARP (aka GPS) is a shared proving service hosted by Starkware.
pub struct SharpProverService {
    sharp_client: SharpClient,
    fact_checker: FactChecker,
}

#[async_trait]
impl ProverClient for SharpProverService {
    #[tracing::instrument(skip(self, task))]
    async fn submit_task(&self, task: Task) -> Result<String, ProverClientError> {
        match task {
            Task::CairoPie(cairo_pie) => {
                let encoded_pie =
                    starknet_os::sharp::pie::encode_pie_mem(cairo_pie).map_err(ProverClientError::PieEncoding)?;
                let (_, job_key) = self.sharp_client.add_job(&encoded_pie).await?;
                Ok(job_key.to_string())
            }
        }
    }

    #[tracing::instrument(skip(self))]
    async fn get_task_status(&self, job_key: &str, fact: &str) -> Result<TaskStatus, ProverClientError> {
        let job_key = Uuid::from_str(job_key)
            .map_err(|e| ProverClientError::InvalidJobKey(format!("Failed to convert {} to UUID {}", job_key, e)))?;
        let res = self.sharp_client.get_job_status(&job_key).await?;

        match res.status {
            // TODO : We would need to remove the FAILED, UNKNOWN, NOT_CREATED status as it is not in the sharp client
            // response specs : https://docs.google.com/document/d/1-9ggQoYmjqAtLBGNNR2Z5eLreBmlckGYjbVl0khtpU0
            // We are waiting for the official public API spec before making changes
            CairoJobStatus::FAILED => Ok(TaskStatus::Failed(res.error_log.unwrap_or_default())),
            CairoJobStatus::INVALID => {
                Ok(TaskStatus::Failed(format!("Task is invalid: {:?}", res.invalid_reason.unwrap_or_default())))
            }
            CairoJobStatus::UNKNOWN => Ok(TaskStatus::Failed(format!("Task not found: {}", job_key))),
            CairoJobStatus::IN_PROGRESS | CairoJobStatus::NOT_CREATED | CairoJobStatus::PROCESSED => {
                Ok(TaskStatus::Processing)
            }
            CairoJobStatus::ONCHAIN => {
                let fact = B256::from_str(fact).map_err(|e| ProverClientError::FailedToConvertFact(e.to_string()))?;
                if self.fact_checker.is_valid(&fact).await? {
                    Ok(TaskStatus::Succeeded)
                } else {
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

    pub fn new_with_settings(settings: &impl Settings) -> Self {
        let sharp_config = SharpConfig::new_with_settings(settings)
            .expect("Not able to create SharpProverService from given settings.");
        let sharp_client = SharpClient::new_with_settings(sharp_config.service_url, settings);
        let fact_checker = FactChecker::new(sharp_config.rpc_node_url, sharp_config.verifier_address);
        Self::new(sharp_client, fact_checker)
    }

    pub fn with_test_settings(settings: &impl Settings, port: u16) -> Self {
        let sharp_config = SharpConfig::new_with_settings(settings)
            .expect("Not able to create SharpProverService from given settings.");
        let sharp_client =
            SharpClient::new_with_settings(format!("http://127.0.0.1:{}", port).parse().unwrap(), settings);
        let fact_checker = FactChecker::new(sharp_config.rpc_node_url, sharp_config.verifier_address);
        Self::new(sharp_client, fact_checker)
    }
}
