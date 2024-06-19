pub mod client;
pub mod config;
pub mod error;

use std::str::FromStr;

use alloy::primitives::B256;
use async_trait::async_trait;
use gps_fact_checker::fact_info::get_fact_info;
use gps_fact_checker::FactChecker;
use prover_client_interface::{ProverClient, ProverClientError, Task, TaskId, TaskStatus};
use snos::sharp::CairoJobStatus;
use utils::settings::SettingsProvider;
use uuid::Uuid;

use crate::client::SharpClient;
use crate::config::SharpConfig;
use crate::error::SharpError;

pub const SHARP_SETTINGS_NAME: &str = "sharp";

/// SHARP (aka GPS) is a shared proving service hosted by Starkware.
pub struct SharpProverService {
    sharp_client: SharpClient,
    fact_checker: FactChecker,
}

#[async_trait]
impl ProverClient for SharpProverService {
    async fn submit_task(&self, task: Task) -> Result<TaskId, ProverClientError> {
        match task {
            Task::CairoPie(cairo_pie) => {
                let fact_info = get_fact_info(&cairo_pie, None)?;
                let encoded_pie =
                    snos::sharp::pie::encode_pie_mem(cairo_pie).map_err(ProverClientError::PieEncoding)?;
                let res = self.sharp_client.add_job(&encoded_pie).await?;
                if let Some(job_key) = res.cairo_job_key {
                    Ok(combine_task_id(&job_key, &fact_info.fact))
                } else {
                    Err(ProverClientError::TaskInvalid(res.error_message.unwrap_or_default()))
                }
            }
        }
    }

    async fn get_task_status(&self, task_id: &TaskId) -> Result<TaskStatus, ProverClientError> {
        let (job_key, fact) = split_task_id(task_id)?;
        let res = self.sharp_client.get_job_status(&job_key).await?;
        match res.status {
            CairoJobStatus::FAILED => Ok(TaskStatus::Failed(res.error_log.unwrap_or_default())),
            CairoJobStatus::INVALID => {
                Ok(TaskStatus::Failed(format!("Task is invalid: {:?}", res.invalid_reason.unwrap_or_default())))
            }
            CairoJobStatus::UNKNOWN => Ok(TaskStatus::Failed(format!("Task not found: {}", task_id))),
            CairoJobStatus::IN_PROGRESS | CairoJobStatus::NOT_CREATED | CairoJobStatus::PROCESSED => {
                Ok(TaskStatus::Processing)
            }
            CairoJobStatus::ONCHAIN => {
                if self.fact_checker.is_valid(&fact).await? {
                    Ok(TaskStatus::Succeeded)
                } else {
                    Ok(TaskStatus::Failed(format!("Fact {} is not valid or not registed", hex::encode(fact))))
                }
            }
        }
    }
}

impl SharpProverService {
    pub fn new(sharp_client: SharpClient, fact_checker: FactChecker) -> Self {
        Self { sharp_client, fact_checker }
    }

    pub fn with_settings(settings: &impl SettingsProvider) -> Self {
        let sharp_cfg: SharpConfig = settings.get_settings(SHARP_SETTINGS_NAME).unwrap();
        let sharp_client = SharpClient::new(sharp_cfg.service_url);
        let fact_checker = FactChecker::new(sharp_cfg.rpc_node_url, sharp_cfg.verifier_address);
        Self::new(sharp_client, fact_checker)
    }
}

/// Construct SHARP specific task ID from job key and proof fact
pub fn combine_task_id(job_key: &Uuid, fact: &B256) -> TaskId {
    format!("{}:{}", job_key, fact)
}

/// Split task ID into SHARP job key and proof fact
pub fn split_task_id(task_id: &TaskId) -> Result<(Uuid, B256), SharpError> {
    let (job_key_str, fact_str) = task_id.split_once(':').ok_or(SharpError::TaskIdSplit)?;
    let job_key = Uuid::from_str(job_key_str).map_err(SharpError::JobKeyParse)?;
    let fact = B256::from_str(fact_str).map_err(SharpError::FactParse)?;
    Ok((job_key, fact))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use cairo_vm::vm::runners::cairo_pie::CairoPie;
    use prover_client_interface::{ProverClient, Task, TaskStatus};
    use tracing::log::log;
    use tracing::log::Level::{Error, Info};
    use utils::settings::default::DefaultSettingsProvider;

    use crate::SharpProverService;

    #[ignore]
    #[tokio::test]
    async fn sharp_reproduce_rate_limiting_issue() {
        // TODO: leaving this test to check if the issue still reproduces (504 error after 8th job status
        // query)
        let sharp_service = SharpProverService::with_settings(&DefaultSettingsProvider {});
        let cairo_pie_path: PathBuf =
            [env!("CARGO_MANIFEST_DIR"), "tests", "artifacts", "fibonacci.zip"].iter().collect();
        let cairo_pie = CairoPie::read_zip_file(&cairo_pie_path).unwrap();
        // Submit task to the testnet prover
        let task_id = sharp_service.submit_task(Task::CairoPie(cairo_pie)).await.unwrap();
        log!(Info, "SHARP: task {} submitted", task_id);
        for attempt in 0..10 {
            tokio::time::sleep(Duration::from_millis((attempt + 1) * 1000)).await;
            match sharp_service.get_task_status(&task_id).await.unwrap() {
                TaskStatus::Failed(err) => {
                    log!(Error, "SHARP: task failed with {}", err);
                    panic!("Task failed");
                }
                TaskStatus::Processing => {
                    log!(Info, "SHARP: task is processing (attempt {})", attempt);
                    continue;
                }
                TaskStatus::Succeeded => {
                    log!(Info, "SHARP: task is completed");
                    return;
                }
            }
        }
        log!(Error, "SHARP: waiting timeout");
        panic!("Out of attempts");
    }
}
