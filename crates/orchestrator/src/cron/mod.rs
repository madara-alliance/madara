use async_trait::async_trait;
use lazy_static::lazy_static;

use crate::queue::job_queue::{WorkerTriggerMessage, WorkerTriggerType};

pub mod event_bridge;

lazy_static! {
    pub static ref WORKER_TRIGGERS: Vec<WorkerTriggerType> = vec![
        WorkerTriggerType::Snos,
        WorkerTriggerType::Proving,
        WorkerTriggerType::DataSubmission,
        WorkerTriggerType::UpdateState
    ];
}

#[async_trait]
pub trait Cron {
    async fn create_cron(&self) -> color_eyre::Result<()>;
    async fn add_cron_target_queue(&self, message: String) -> color_eyre::Result<()>;
    async fn setup(&self) -> color_eyre::Result<()> {
        self.create_cron().await?;
        for triggers in WORKER_TRIGGERS.iter() {
            self.add_cron_target_queue(get_worker_trigger_message(triggers.clone())?).await?;
        }
        Ok(())
    }
}

fn get_worker_trigger_message(worker_trigger_type: WorkerTriggerType) -> color_eyre::Result<String> {
    let message = WorkerTriggerMessage { worker: worker_trigger_type };
    Ok(serde_json::to_string(&message)?)
}
