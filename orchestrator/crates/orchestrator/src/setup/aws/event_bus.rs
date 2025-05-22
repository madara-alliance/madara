use crate::cli::cron::event_bridge::EventBridgeType;
use crate::core::client::event_bus::event_bridge::EventBridgeClient;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::jobs::WorkerTriggerType;
use crate::types::params::CronArgs;
use crate::{OrchestratorError, OrchestratorResult};
use async_trait::async_trait;
use lazy_static::lazy_static;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

lazy_static! {
    pub static ref WORKER_TRIGGERS: Vec<WorkerTriggerType> = vec![
        WorkerTriggerType::Snos,
        WorkerTriggerType::Proving,
        WorkerTriggerType::ProofRegistration,
        WorkerTriggerType::DataSubmission,
        WorkerTriggerType::UpdateState
    ];
}

#[async_trait]
impl Resource for EventBridgeClient {
    type SetupResult = ();
    type CheckResult = ();
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = CronArgs;
    type CheckArgs = (EventBridgeType, WorkerTriggerType, String);

    async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config, None)),
        }
    }

    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        let trigger_arns = self
            .create_cron(
                args.target_queue_name.clone(),
                args.trigger_role_name.clone(),
                args.trigger_policy_name.clone(),
            )
            .await
            .map_err(|e| {
                OrchestratorError::SetupCommandError(format!(
                    "Failed to create cron: {:?} for queue: {:?}",
                    e,
                    args.target_queue_name.clone()
                ))
            })?;
        sleep(Duration::from_secs(15)).await;

        for trigger in WORKER_TRIGGERS.iter() {
            if self
                .check_if_exists((args.event_bridge_type.clone(), trigger.clone(), args.trigger_rule_name.clone()))
                .await?
            {
                tracing::info!(" ⏭️ Event Bridge {trigger} already exists, skipping");
            } else {
                self.add_cron_target_queue(
                    trigger,
                    &trigger_arns,
                    args.trigger_rule_name.clone(),
                    args.event_bridge_type.clone(),
                    Duration::from_secs(args.cron_time.clone().parse::<u64>().map_err(|e| {
                        OrchestratorError::SetupCommandError(format!("Failed to parse the cron time: {:?}", e))
                    })?),
                )
                .await
                .expect("Failed to add Event Bus target queue");
            }
        }
        Ok(())
    }
    /// check_if_exists - Check if the event bridge rule exists
    ///
    /// # Arguments
    /// * `args` - The arguments for the check
    ///
    /// # Returns
    /// * `OrchestratorResult<bool>` - A result indicating if the event bridge rule exists
    ///
    async fn check_if_exists(&self, args: Self::CheckArgs) -> OrchestratorResult<bool> {
        let (event_bridge_type, trigger_type, trigger_rule_name) = args;
        let trigger_name = format!("{}-{}", trigger_rule_name.clone(), trigger_type);
        match event_bridge_type {
            EventBridgeType::Rule => Ok(self.eb_client.describe_rule().name(trigger_name).send().await.is_ok()),
            EventBridgeType::Schedule => {
                Ok(self.scheduler_client.get_schedule().name(trigger_name).send().await.is_ok())
            }
        }
    }

    /// is_ready_to_use - Check if the event bridge rule is ready to use
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments for the check
    ///
    /// # Returns
    /// * `OrchestratorResult<bool>` - A result indicating if the event bridge rule is ready to use
    async fn is_ready_to_use(&self, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        Ok(self.eb_client.describe_rule().name(&args.trigger_rule_name).send().await.is_ok())
    }
}
