use std::time::Duration;

use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_eventbridge::types::{InputTransformer, RuleState, Target};
use aws_sdk_eventbridge::Client as EventBridgeClient;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client as SqsClient;

use crate::cron::Cron;

#[derive(Clone, Debug)]
pub struct AWSEventBridgeValidatedArgs {
    pub target_queue_name: String,
    pub cron_time: Duration,
    pub trigger_rule_name: String,
}

pub struct AWSEventBridge {
    target_queue_name: String,
    cron_time: Duration,
    trigger_rule_name: String,
    client: EventBridgeClient,
    queue_client: SqsClient,
}

impl AWSEventBridge {
    pub fn new_with_args(params: &AWSEventBridgeValidatedArgs, aws_config: &SdkConfig) -> Self {
        Self {
            target_queue_name: params.target_queue_name.clone(),
            cron_time: params.cron_time,
            trigger_rule_name: params.trigger_rule_name.clone(),
            client: aws_sdk_eventbridge::Client::new(aws_config),
            queue_client: aws_sdk_sqs::Client::new(aws_config),
        }
    }
}

#[async_trait]
#[allow(unreachable_patterns)]
impl Cron for AWSEventBridge {
    async fn create_cron(&self) -> color_eyre::Result<()> {
        self.client
            .put_rule()
            .name(&self.trigger_rule_name)
            .schedule_expression(duration_to_rate_string(self.cron_time))
            .state(RuleState::Enabled)
            .send()
            .await?;

        Ok(())
    }
    async fn add_cron_target_queue(&self, message: String) -> color_eyre::Result<()> {
        let queue_url = self.queue_client.get_queue_url().queue_name(&self.target_queue_name).send().await?;

        let queue_attributes = self
            .queue_client
            .get_queue_attributes()
            .queue_url(queue_url.queue_url.unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;
        let queue_arn = queue_attributes.attributes().unwrap().get(&QueueAttributeName::QueueArn).unwrap();

        // Create the EventBridge target with the input transformer
        let input_transformer =
            InputTransformer::builder().input_paths_map("$.time", "time").input_template(message).build()?;

        self.client
            .put_targets()
            .rule(&self.trigger_rule_name)
            .targets(
                Target::builder()
                    .id(uuid::Uuid::new_v4().to_string())
                    .arn(queue_arn)
                    .input_transformer(input_transformer)
                    .build()?,
            )
            .send()
            .await?;

        Ok(())
    }
}

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

#[cfg(test)]
mod event_bridge_utils_test {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_duration_to_rate_string() {
        assert_eq!(duration_to_rate_string(Duration::from_secs(60)), "rate(1 minute)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(120)), "rate(2 minutes)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(30)), "rate(30 seconds)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(3600)), "rate(1 hour)");
        assert_eq!(duration_to_rate_string(Duration::from_secs(86400)), "rate(1 day)");
    }
}
