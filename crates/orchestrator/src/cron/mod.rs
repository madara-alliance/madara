use std::time::Duration;

use async_trait::async_trait;
use lazy_static::lazy_static;

use crate::queue::job_queue::{WorkerTriggerMessage, WorkerTriggerType};
use crate::setup::SetupConfig;

pub mod event_bridge;

lazy_static! {
    pub static ref CRON_DURATION: Duration = Duration::from_mins(1);
    // TODO : we can take this from clap.
    pub static ref TARGET_QUEUE_NAME: String = String::from("madara_orchestrator_worker_trigger_queue");
    pub static ref WORKER_TRIGGERS: Vec<WorkerTriggerType> = vec![
        WorkerTriggerType::Snos,
        WorkerTriggerType::Proving,
        WorkerTriggerType::DataSubmission,
        WorkerTriggerType::UpdateState
    ];
    pub static ref WORKER_TRIGGER_RULE_NAME: String = String::from("worker_trigger_scheduled");
}

#[async_trait]
pub trait Cron {
    async fn create_cron(
        &self,
        config: &SetupConfig,
        cron_time: Duration,
        trigger_rule_name: String,
    ) -> color_eyre::Result<()>;
    async fn add_cron_target_queue(
        &self,
        config: &SetupConfig,
        target_queue_name: String,
        message: String,
        trigger_rule_name: String,
    ) -> color_eyre::Result<()>;
    async fn setup(&self, config: SetupConfig) -> color_eyre::Result<()> {
        self.create_cron(&config, *CRON_DURATION, WORKER_TRIGGER_RULE_NAME.clone()).await?;
        for triggers in WORKER_TRIGGERS.iter() {
            self.add_cron_target_queue(
                &config,
                TARGET_QUEUE_NAME.clone(),
                get_worker_trigger_message(triggers.clone())?,
                WORKER_TRIGGER_RULE_NAME.clone(),
            )
            .await?;
        }
        Ok(())
    }
}

fn get_worker_trigger_message(worker_trigger_type: WorkerTriggerType) -> color_eyre::Result<String> {
    let message = WorkerTriggerMessage { worker: worker_trigger_type };
    Ok(serde_json::to_string(&message)?)
}
