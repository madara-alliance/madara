use crate::core::client::event_bus::event_bridge::EventBridgeClient;
use crate::core::client::SNS;
use crate::core::traits::resource::Resource;
use crate::setup::creator::{EventBridgeResourceCreator, ResourceCreator, ResourceType, S3ResourceCreator, SNSResourceCreator, SQSResourceCreator};
use crate::{
    core::client::storage::s3::AWSS3,
    core::client::SQS,
    core::cloud::CloudProvider,
    types::params::{AlertArgs, CronArgs, QueueArgs, StorageArgs},
    OrchestratorResult,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// ResourceFactory is responsible for creating resources based on their type
pub struct ResourceFactory {
    creators: HashMap<ResourceType, Box<dyn ResourceCreator>>,
    cloud_provider: Arc<CloudProvider>,
    queue_params: QueueArgs,
    cron_params: CronArgs,
    storage_params: StorageArgs,
    alert_params: AlertArgs,
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
    ) -> Self {
        let mut creators = HashMap::new();
        creators.insert(ResourceType::Storage, Box::new(S3ResourceCreator) as Box<dyn ResourceCreator>);
        creators.insert(ResourceType::Queue, Box::new(SQSResourceCreator) as Box<dyn ResourceCreator>);
        creators.insert(ResourceType::EventBus, Box::new(EventBridgeResourceCreator) as Box<dyn ResourceCreator>);
        creators.insert(ResourceType::PubSub, Box::new(SNSResourceCreator) as Box<dyn ResourceCreator>);

        ResourceFactory { creators, cloud_provider, queue_params, cron_params, storage_params, alert_params }
    }

    pub async fn setup_resource(&self) -> OrchestratorResult<()> {
        let mut resource_futures = Vec::new();
        for (resource_type, creator) in self.creators.iter() {
            info!(" ⏳ Setting up resource: {:?}", resource_type);
            let mut resource = creator.create_resource_client(self.cloud_provider.clone()).await?;
            let is_queue_ready = Arc::new(tokio::sync::RwLock::new(false));
            let is_queue_ready_clone = is_queue_ready.clone();

            let storage_params = self.storage_params.clone();
            let queue_params = self.queue_params.clone();
            let alert_params = self.alert_params.clone();
            let cron_params = self.cron_params.clone();
            let resource_type = resource_type.clone();

            let resource_future = async move {
                let result: OrchestratorResult<()> = async {
                    match resource_type {
                        ResourceType::Storage => {
                            let rs = resource.downcast_mut::<AWSS3>().unwrap();
                            rs.setup(storage_params).await?;
                            Ok(())
                        }
                        ResourceType::Queue => {
                            let rs = resource.downcast_mut::<SQS>().unwrap();
                            rs.setup(queue_params.clone()).await?;
                            let queue_ready = rs
                                .poll(
                                    queue_params,
                                    5,
                                    6,
                                )
                                .await;
                            *is_queue_ready_clone.write().await = queue_ready;
                            Ok(())
                        }
                        ResourceType::PubSub => {
                            let rs = resource.downcast_mut::<SNS>().unwrap();
                            rs.setup(alert_params).await?;
                            Ok(())
                        }
                        ResourceType::EventBus => {
                            let rs = resource.downcast_mut::<EventBridgeClient>().unwrap();
                            rs.setup(cron_params).await?;
                            Ok(())
                        }
                    }
                }.await;

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
