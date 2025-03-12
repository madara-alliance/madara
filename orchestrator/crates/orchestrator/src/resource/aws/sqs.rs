use crate::core::cloud::CloudProvider;
use crate::error::{OrchestratorError, OrchestratorResult};
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
    pub queue_name: String,
    pub visibility_timeout: u32,
}

impl SQS {
    // Helper to construct a full SQS queue URL from the base URL and queue name
    pub fn get_queue_url(base_url: &str, queue_name: &str) -> String {
        format!("{}/{}", base_url, queue_name)
    }
}

#[async_trait]
impl Resource for SQS {
    type SetupResult = SQSSetupArgs;
    type CheckResult = ();
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = SQSSetupArgs;

    type CheckArgs = ();

    async fn new(cloud_provider: CloudProvider) -> OrchestratorResult<Self> {
        match cloud_provider {
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
        let exists_q = &self
            .client
            .get_queue_url()
            .queue_name(&args.queue_name)
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to list queue: {}", e)))?;

        // If the queue already exists, just use it
        let queue_url = if let Some(url) = &exists_q.queue_url {
            url.clone()
        } else {
            // Create a new queue
            let queue = self.client.create_queue().queue_name(&args.queue_name).send().await.map_err(|e| {
                OrchestratorError::ResourceSetupError(format!(
                    "Failed to create SQS queue '{}': {}",
                    args.queue_name, e
                ))
            })?;

            queue
                .queue_url()
                .ok_or_else(|| OrchestratorError::ResourceSetupError("Failed to get queue url".to_string()))?
                .to_string()
        };

        // Store the queue URL in self using Arc<Mutex<>>
        if let Ok(mut queue_url_guard) = self.queue_url.lock() {
            *queue_url_guard = Some(queue_url.clone());
        }

        let mut attributes = HashMap::new();
        attributes.insert(QueueAttributeName::VisibilityTimeout, args.visibility_timeout.to_string());
        self.client
            .set_queue_attributes()
            .queue_url(&queue_url)
            .set_attributes(Some(attributes))
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to set queue attributes: {}", e)))?;

        Ok(args)
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
        let args = SQSSetupArgs { queue_name: "test-queue".to_string(), visibility_timeout: 30 };

        assert_eq!(args.queue_name, "test-queue");
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
