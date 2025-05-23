use crate::cli::cron::event_bridge::EventBridgeType;
use crate::cli::Layer;
use crate::core::client::event_bus::event_bridge::InnerAWSEventBridge;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::jobs::WorkerTriggerType;
use crate::types::params::CronArgs;
use crate::{OrchestratorError, OrchestratorResult};
use anyhow::Error;
use async_trait::async_trait;
use aws_sdk_eventbridge::types::{InputTransformer, RuleState, Target as EventBridgeTarget};
use aws_sdk_scheduler::types::{FlexibleTimeWindow, FlexibleTimeWindowMode, Target};
use aws_sdk_sqs::types::QueueAttributeName;
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

#[derive(Debug, Clone)]
pub struct TriggerArns {
    queue_arn: String,
    role_arn: String,
}

// TODO: Ideally I should automatically get the TARGET_QUEUE_NAME from queue params

#[async_trait]
impl Resource for InnerAWSEventBridge {
    type SetupResult = ();
    type CheckResult = ();
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = CronArgs;
    type CheckArgs = (EventBridgeType, WorkerTriggerType, String);

    async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config)),
        }
    }

    async fn setup(&self, layer: Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
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
            // Proof registration is only required in L3
            // TODO: Remove this once we have handle the pipeline with state machine
            if *trigger == WorkerTriggerType::ProofRegistration && layer != Layer::L3 {
                continue;
            }
            if self
                .check_if_exists(&(args.event_bridge_type.clone(), trigger.clone(), args.trigger_rule_name.clone()))
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
    async fn check_if_exists(&self, args: &Self::CheckArgs) -> OrchestratorResult<bool> {
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

impl InnerAWSEventBridge {
    /// get_queue_arn - Get the ARN of a queue
    ///
    /// # Arguments
    ///
    /// * `queue_name` - The name of the queue
    ///
    /// # Returns
    /// * `String` - The ARN of the queue
    async fn get_queue_arn(&self, queue_name: &str) -> Result<String, Error> {
        let queue_url = self.queue_client.get_queue_url().queue_name(queue_name).send().await?;
        let queue_attributes = self
            .queue_client
            .get_queue_attributes()
            .queue_url(queue_url.queue_url.unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;
        queue_attributes
            .attributes()
            .and_then(|attrs| attrs.get(&QueueAttributeName::QueueArn))
            .map(String::from)
            .ok_or_else(|| Error::msg("Queue ARN not found"))
    }

    async fn create_iam_role(&self, role_name: &str) -> Result<String, Error> {
        let assume_role_policy = r#"{
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Principal": {
                    "Service": ["scheduler.amazonaws.com", "events.amazonaws.com"]
                },
                "Action": "sts:AssumeRole"
            }]
        }"#;
        let create_role_resp = self
            .iam_client
            .create_role()
            .role_name(role_name)
            .assume_role_policy_document(assume_role_policy)
            .send()
            .await?;
        let role = create_role_resp.role().ok_or_else(|| Error::msg("Failed to create IAM role"))?;
        Ok(role.arn().to_string())
    }

    async fn create_and_attach_sqs_policy(
        &self,
        policy_name: &str,
        role_name: &str,
        queue_arn: &str,
    ) -> Result<(), Error> {
        let policy_document = format!(
            r#"{{
            "Version": "2012-10-17",
            "Statement": [{{
                "Effect": "Allow",
                "Action": ["sqs:SendMessage"],
                "Resource": "{}"
            }}]
        }}"#,
            queue_arn
        );
        let create_policy_resp =
            self.iam_client.create_policy().policy_name(policy_name).policy_document(&policy_document).send().await?;
        let policy = create_policy_resp.policy().ok_or_else(|| Error::msg("Failed to create policy"))?;

        let policy_arn = policy.arn().ok_or_else(|| Error::msg("Failed to get policy ARN"))?;

        self.iam_client.attach_role_policy().role_name(role_name).policy_arn(policy_arn).send().await?;

        Ok(())
    }

    pub async fn create_cron(
        &self,
        target_queue_name: String,
        trigger_role_name: String,
        trigger_policy_name: String,
    ) -> Result<TriggerArns, Error> {
        let queue_arn = self.get_queue_arn(&target_queue_name).await?;

        let role_name = format!("{}-{}", trigger_role_name, uuid::Uuid::new_v4());
        let role_arn = self.create_iam_role(&role_name).await?;

        let policy_name = format!("{}-{}", trigger_policy_name, uuid::Uuid::new_v4());
        self.create_and_attach_sqs_policy(&policy_name, &role_name, &queue_arn).await?;

        Ok(TriggerArns { queue_arn, role_arn })
    }

    /// duration_to_rate_string - Converts a Duration to a rate string for AWS EventBridge
    ///
    /// # Arguments
    ///
    /// * `duration` - The duration to convert
    ///
    /// # Returns
    ///
    /// * `String` - The rate string in the format "rate(X unit)" where X is the number of units
    ///
    fn duration_to_rate_string(duration: Duration) -> String {
        let total_secs = duration.as_secs();
        let total_mins = duration.as_secs() / 60;
        let total_hours = duration.as_secs() / 3600;
        let total_days = duration.as_secs() / 86400;

        if total_days > 0 {
            format!("rate({} day{})", total_days, if total_days == 1 { "" } else { "s" })
        } else if total_hours > 0 {
            format!("rate({} hour{})", total_hours, if total_hours == 1 { "" } else { "s" })
        } else if total_mins > 0 {
            format!("rate({} minute{})", total_mins, if total_mins == 1 { "" } else { "s" })
        } else {
            format!("rate({} second{})", total_secs, if total_secs == 1 { "" } else { "s" })
        }
    }
    /// TODO: we might need to move this code to Setup since there is not use case for this function in client
    pub(crate) async fn add_cron_target_queue(
        &self,
        trigger_type: &WorkerTriggerType,
        trigger_arns: &TriggerArns,
        trigger_rule_name: String,
        event_bridge_type: EventBridgeType,
        cron_time: Duration,
    ) -> color_eyre::Result<()> {
        let message = trigger_type.clone().to_string();
        let trigger_name = format!("{}-{}", trigger_rule_name.clone(), trigger_type);

        match event_bridge_type.clone() {
            EventBridgeType::Rule => {
                let input_transformer =
                    InputTransformer::builder().input_paths_map("time", "$.time").input_template(message).build()?;

                self.eb_client
                    .put_rule()
                    .name(trigger_name.clone())
                    .schedule_expression("rate(1 minute)")
                    .state(RuleState::Enabled)
                    .send()
                    .await?;

                self.eb_client
                    .put_targets()
                    .rule(trigger_name.clone())
                    .targets(
                        EventBridgeTarget::builder()
                            .id(uuid::Uuid::new_v4().to_string())
                            .arn(trigger_arns.queue_arn.clone())
                            .input_transformer(input_transformer.clone())
                            .build()?,
                    )
                    .send()
                    .await?;
            }
            EventBridgeType::Schedule => {
                // Set flexible time window (you can adjust this as needed)
                let flexible_time_window = FlexibleTimeWindow::builder().mode(FlexibleTimeWindowMode::Off).build()?;

                let message = trigger_type.clone().to_string();

                // Create target for SQS queue
                let target = Target::builder()
                    .arn(trigger_arns.queue_arn.clone())
                    .role_arn(trigger_arns.role_arn.clone())
                    .input(message)
                    .build()?;

                // Create the schedule
                self.scheduler_client
                    .create_schedule()
                    .name(trigger_name)
                    .schedule_expression_timezone("UTC")
                    .flexible_time_window(flexible_time_window)
                    .schedule_expression(Self::duration_to_rate_string(cron_time))
                    .target(target)
                    .send()
                    .await?;
            }
        };

        Ok(())
    }
}
