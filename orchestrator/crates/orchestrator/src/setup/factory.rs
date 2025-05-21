use crate::core::client::event_bus::event_bridge::EventBridgeClient;

use crate::core::traits::resource::Resource;
use crate::setup::creator::{
    ResourceType,
    // EventBridgeResourceCreator, ResourceCreator,
    // S3ResourceCreator, SNSResourceCreator,
    // SQSResourceCreator,
};
use crate::types::params::MiscellaneousArgs;
use crate::{
    core::client::AWSS3,
    core::client::SNS,
    core::client::SQS,
    core::cloud::CloudProvider,
    types::params::{AlertArgs, CronArgs, QueueArgs, StorageArgs},
    OrchestratorError, OrchestratorResult,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// ResourceFactory is responsible for creating resources based on their type
pub struct ResourceFactory {
    // Vec to maintain order of insertion
    ordered_types: Vec<ResourceType>,
    cloud_provider: Arc<CloudProvider>,
    queue_params: QueueArgs,
    cron_params: CronArgs,
    storage_params: StorageArgs,
    alert_params: AlertArgs,
    miscellaneous_params: MiscellaneousArgs,
}

impl ResourceFactory {
    /// new_with_aws - Create a new ResourceFactory with default resource creators for Orchestrator
    /// with AWS Cloud Provider
    ///
    /// # Arguments
    /// * `cloud_provider` - The cloud provider to use for resource creation
    /// * `queue_params` - The parameters for the queue resource
    /// * `cron_params` - The parameters for the cron resource
    /// * `storage_params` - The parameters for the storage resource
    /// * `alert_params` - The parameters for the alert resource
    ///
    /// # Returns
    /// * `ResourceFactory` - A new instance of ResourceFactory
    ///
    pub fn new_with_aws(
        cloud_provider: Arc<CloudProvider>,
        queue_params: QueueArgs,
        cron_params: CronArgs,
        storage_params: StorageArgs,
        alert_params: AlertArgs,
        miscellaneous_params: MiscellaneousArgs,
    ) -> Self {
        let ordered_types = vec![
            // (ResourceType::Storage, Box::new(S3ResourceCreator) as Box<dyn ResourceCreator>),
            // (ResourceType::Queue, Box::new(SQSResourceCreator) as Box<dyn ResourceCreator>),
            // (ResourceType::EventBus, Box::new(EventBridgeResourceCreator) as Box<dyn ResourceCreator>),
            // (ResourceType::PubSub, Box::new(SNSResourceCreator) as Box<dyn ResourceCreator>),
            ResourceType::Storage,
            ResourceType::Queue,
            ResourceType::EventBus,
            ResourceType::PubSub,
        ];

        ResourceFactory {
            ordered_types,
            cloud_provider,
            queue_params,
            cron_params,
            storage_params,
            alert_params,
            miscellaneous_params,
        }
    }

    /// setup_resource - Set up the resources in the factory
    /// NOTE: this function length is a bit long, but it is necessary to maintain the order of resource creation
    /// in the future, we can refactor this function to use a more generic approach when we add more cloud providers
    /// TODO > Refactor this function to use a more generic approach when we add more cloud providers
    pub async fn setup_resource(&self) -> OrchestratorResult<()> {
        let mut resource_futures = Vec::new();
        let is_queue_ready = Arc::new(AtomicBool::new(false));
        // Use ordered_types to maintain creation order
        for resource_type in self.ordered_types.iter() {
            info!(" ⏳ Setting up resource: {:?}", resource_type);
            // There's no actual benefit of creating the resource here, it's leading to a major Option handling in the indivudual client,
            // wherein it's been validated in the fn that calls setup_resource that all the params have been validated to exist.
            // So, to simplify the code cycle, we are shifting the resource creation to the resource_future async move
            // And are providing it with it's appropriate params.
            let is_queue_ready_clone = is_queue_ready.clone();

            // when we have multiple providers, these all should also be a enum of provider params.
            let storage_params = self.storage_params.clone();
            let queue_params = self.queue_params.clone();
            let alert_params = self.alert_params.clone();
            let cron_params = self.cron_params.clone();
            let resource_type = resource_type.clone();
            let miscellaneous_params = self.miscellaneous_params.clone();

            let resource_future = async move {
                let result: OrchestratorResult<()> = async {
                    match resource_type {
                        ResourceType::Storage => {
                            let rs = match self.cloud_provider.as_ref() {
                                CloudProvider::AWS(aws_config) => {
                                    let s3 = AWSS3::new(aws_config, &self.storage_params);
                                    Ok::<_, OrchestratorError>(s3)
                                }?,
                            };

                            rs.setup().await?;
                            rs.poll(storage_params, miscellaneous_params.poll_interval, miscellaneous_params.timeout)
                                .await;
                            Ok(())
                        }
                        ResourceType::Queue => {
                            let rs = match self.cloud_provider.as_ref() {
                                CloudProvider::AWS(aws_config) => {
                                    let sqs = SQS::new(aws_config, &self.queue_params);
                                    Ok::<_, OrchestratorError>(sqs)
                                }?,
                            };
                            rs.setup().await?;

                            let queue_ready = rs
                                .poll(queue_params, miscellaneous_params.poll_interval, miscellaneous_params.timeout)
                                .await;
                            is_queue_ready_clone.store(queue_ready, Ordering::Release);
                            Ok(())
                        }
                        ResourceType::PubSub => {
                            let start_time = std::time::Instant::now();
                            let timeout_duration = Duration::from_secs(miscellaneous_params.timeout);
                            let poll_duration = Duration::from_secs(miscellaneous_params.poll_interval);

                            while start_time.elapsed() < timeout_duration {
                                if is_queue_ready_clone.load(Ordering::Acquire) {
                                    info!(" ✅ Queue is ready, setting up SNS");
                                    let rs = match self.cloud_provider.as_ref() {
                                        CloudProvider::AWS(aws_config) => {
                                            let sns = SNS::new(aws_config, &self.alert_params);
                                            Ok::<_, OrchestratorError>(sns)
                                        }?,
                                    };

                                    rs.setup().await?;
                                    rs.poll(
                                        alert_params,
                                        miscellaneous_params.poll_interval,
                                        miscellaneous_params.timeout,
                                    )
                                    .await;
                                    break;
                                } else {
                                    info!(" Current Status of the Queue Creation is: {:?}", is_queue_ready_clone);
                                    info!(" ⏳ Waiting for queues to be ready before setting up SNS");
                                    tokio::time::sleep(poll_duration).await;
                                }
                            }
                            Ok(())
                        }
                        ResourceType::EventBus => {
                            let start_time = std::time::Instant::now();
                            let timeout_duration = Duration::from_secs(miscellaneous_params.timeout);
                            let poll_duration = Duration::from_secs(miscellaneous_params.poll_interval);

                            while start_time.elapsed() < timeout_duration {
                                if is_queue_ready_clone.load(Ordering::Acquire) {
                                    info!(" ✅ Queue is ready, setting up EventBridge");
                                    let rs = match self.cloud_provider.as_ref() {
                                        CloudProvider::AWS(aws_config) => {
                                            let eb = EventBridgeClient::new(aws_config, Some(&self.cron_params));
                                            Ok::<_, OrchestratorError>(eb)
                                        }?,
                                    };
                                    rs.setup().await?;
                                    break;
                                } else {
                                    info!(" Current Status of the Queue Creation is: {:?}", is_queue_ready_clone);
                                    info!(" ⏳ Waiting for queues to be ready before setting up EventBridge");
                                    tokio::time::sleep(poll_duration).await;
                                }
                            }
                            Ok(())
                        }
                    }
                }
                .await;

                if let Err(ref e) = result {
                    info!(" ❌ Resource setup failed for {:?}: {:?}", resource_type, e);
                } else {
                    info!(" ✅ Resource setup completed: {:?}", resource_type);
                }
                result
            };
            resource_futures.push(resource_future);
        }

        let results = futures::future::join_all(resource_futures).await;
        for result in results {
            result?;
        }
        Ok(())
    }
}
