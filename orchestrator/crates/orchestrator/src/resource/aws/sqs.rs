use crate::core::cloud::CloudProvider;
use crate::core::QueueType;
use crate::error::{OrchestratorError, OrchestratorResult};
use crate::params::QueueArgs;
use crate::resource::config::QUEUES;
use crate::resource::Resource;
use async_trait::async_trait;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct SQS {
    pub client: Client,
    pub queue_url: Arc<Mutex<Option<String>>>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

pub struct SQSSetupArgs {
    pub name: String,
    pub visibility_timeout: u32,
}

impl SQS {
    // Helper to construct a full SQS queue URL from the base URL and queue name
    pub fn get_queue_url(base_url: &str, queue_name: &str) -> String {
        format!("{}/{}", base_url, queue_name)
    }

    pub fn get_queue_name(&self, sqs_prefix: String, queue_type: QueueType, sqs_suffix: String) -> String {
        format!("{}_{:?}_{}", sqs_prefix, queue_type, sqs_suffix)
    }
    /// To get the queue url from the given queue name
    async fn get_queue_url_from_client(queue_name: &str, sqs_client: &Client) -> OrchestratorResult<String> {
        Ok(sqs_client
            .get_queue_url()
            .queue_name(queue_name)
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceError(format!("Failed to get queue url: {}", e)))?
            .queue_url()
            .ok_or_else(|| {
                OrchestratorError::ResourceError("Unable to get queue url from the given queue_name.".to_string())
            })?
            .to_string())
    }

    async fn get_queue_arn(client: &Client, queue_url: &str) -> OrchestratorResult<String> {
        let attributes = client
            .get_queue_attributes()
            .queue_url(queue_url)
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceError(format!("Failed to get queue attributes: {}", e)))?;

        Ok(attributes.attributes().unwrap().get(&QueueAttributeName::QueueArn).unwrap().to_string())
    }
}

#[async_trait]
impl Resource for SQS {
    type SetupResult = ();
    type CheckResult = ();
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = QueueArgs;

    type CheckArgs = ();

    async fn new(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => {
                let client = Client::new(&aws_config);
                Ok(Self { client, queue_url: Arc::new(Mutex::new(None)), prefix: None, suffix: None })
            }
            _ => Err(OrchestratorError::InvalidCloudProviderError(format!(
                "Miss match Cloud Provider {:?}",
                cloud_provider
            ))),
        }
    }

    /// setup SQS queue
    /// check if queue exists, if not create it
    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        for queue in QUEUES.iter() {
            let res = self
                .client
                .create_queue()
                .queue_name(self.get_queue_name(args.prefix.clone(), queue.name.clone(), args.suffix.clone()))
                .send()
                .await
                .map_err(|e| {
                    OrchestratorError::ResourceSetupError(format!(
                        "Failed to create SQS queue '{}': {}",
                        args.queue_base_url, e
                    ))
                })?;
            let queue_url = res
                .queue_url()
                .ok_or_else(|| OrchestratorError::ResourceSetupError("Failed to get queue url".to_string()))?;

            let mut attributes = HashMap::new();
            attributes.insert(QueueAttributeName::VisibilityTimeout, queue.visibility_timeout.to_string());

            if let Some(dlq_config) = &queue.dlq_config {
                let dlq_url = Self::get_queue_url_from_client(
                    self.get_queue_name(args.prefix.clone(), dlq_config.dlq_name.clone(), args.suffix.clone()).as_str(),
                    &self.client,
                )
                .await?;
                let dlq_arn = Self::get_queue_arn(&self.client, &dlq_url).await?;
                let policy = format!(
                    r#"{{"deadLetterTargetArn":"{}","maxReceiveCount":"{}"}}"#,
                    dlq_arn, &dlq_config.max_receive_count
                );
                attributes.insert(QueueAttributeName::RedrivePolicy, policy);
            }

            self.client.set_queue_attributes().queue_url(queue_url).set_attributes(Some(attributes)).send().await?;
        }

        Ok(())
    }

    async fn check(&self, _args: Self::CheckArgs) -> OrchestratorResult<Self::CheckResult> {
        let queue_url = if let Ok(queue_url_guard) = self.queue_url.lock() {
            queue_url_guard.clone()
        } else {
            return Err(OrchestratorError::ResourceError("Failed to access queue URL".to_string()));
        };

        if let Some(queue_url) = queue_url {
            // Try to get queue attributes to verify queue exists
            let _result = self
                .client
                .get_queue_attributes()
                .queue_url(&queue_url)
                .attribute_names(QueueAttributeName::All)
                .send()
                .await
                .map_err(|e| OrchestratorError::ResourceError(format!("Failed to check SQS queue: {}", e)))?;

            // If we get attributes back, the queue exists
            return Ok(());
        }

        // If queue_url is None, we don't have a queue to check
        Err(OrchestratorError::ResourceError("No queue URL to check".to_string()))
    }

    async fn teardown(&self) -> OrchestratorResult<()> {
        let queue_url = if let Ok(queue_url_guard) = self.queue_url.lock() {
            queue_url_guard.clone()
        } else {
            return Err(OrchestratorError::ResourceError("Failed to access queue URL".to_string()));
        };

        if let Some(queue_url) = queue_url {
            // Delete the queue
            self.client
                .delete_queue()
                .queue_url(&queue_url)
                .send()
                .await
                .map_err(|e| OrchestratorError::ResourceError(format!("Failed to delete SQS queue: {}", e)))?;

            // Clear the queue URL
            if let Ok(mut queue_url_guard) = self.queue_url.lock() {
                *queue_url_guard = None;
            }

            return Ok(());
        }

        // If queue_url is None, there's nothing to tear down
        Err(OrchestratorError::ResourceError("No queue URL to tear down".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_setup_args() {
        let args = SQSSetupArgs { name: "test-queue".to_string(), visibility_timeout: 30 };

        assert_eq!(args.name, "test-queue");
        assert_eq!(args.visibility_timeout, 30);
    }

    #[test]
    fn test_queue_url_storage() {
        let sqs = SQS {
            client: Client::new(&aws_config::SdkConfig::builder().build()),
            queue_url: Arc::new(Mutex::new(None)),
            prefix: None,
            suffix: None,
        };

        // Test setting the queue URL
        {
            let mut queue_url_guard = sqs.queue_url.lock().unwrap();
            *queue_url_guard = Some("https://sqs.us-east-1.amazonaws.com/123456789012/test-queue".to_string());
        }

        // Test getting the queue URL
        {
            let queue_url_guard = sqs.queue_url.lock().unwrap();
            assert_eq!(
                *queue_url_guard,
                Some("https://sqs.us-east-1.amazonaws.com/123456789012/test-queue".to_string())
            );
        }
    }
}
