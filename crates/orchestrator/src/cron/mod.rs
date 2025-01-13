use async_trait::async_trait;
use lazy_static::lazy_static;

use crate::queue::job_queue::WorkerTriggerType;

pub mod event_bridge;

lazy_static! {
    pub static ref WORKER_TRIGGERS: Vec<WorkerTriggerType> = vec![
        WorkerTriggerType::Snos,
        WorkerTriggerType::Proving,
        WorkerTriggerType::DataSubmission,
        WorkerTriggerType::UpdateState
    ];
}

#[derive(Debug, Clone)]
pub struct TriggerArns {
    queue_arn: String,
    role_arn: String,
}
#[async_trait]
pub trait Cron {
    async fn create_cron(&self) -> color_eyre::Result<TriggerArns>;
    async fn add_cron_target_queue(
        &self,
        trigger_type: &WorkerTriggerType,
        trigger_arns: &TriggerArns,
    ) -> color_eyre::Result<()>;
    async fn setup(&self) -> color_eyre::Result<()> {
        let trigger_arns = self.create_cron().await?;
        for trigger in WORKER_TRIGGERS.iter() {
            self.add_cron_target_queue(trigger, &trigger_arns).await?;
        }
        Ok(())
    }
}

pub fn get_worker_trigger_message(worker_trigger_type: WorkerTriggerType) -> color_eyre::Result<String> {
    Ok(worker_trigger_type.to_string())
}
