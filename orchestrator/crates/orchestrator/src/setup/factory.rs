use crate::core::client::cron::event_bridge::EventBridgeClient;
use crate::core::client::SNS;
use crate::core::traits::resource::Resource;
use crate::setup::creator::{
    EventBridgeResourceCreator, ResourceCreator, ResourceType, S3ResourceCreator, SNSResourceCreator,
    SQSResourceCreator,
};
use crate::setup::wrapper::ResourceWrapper;
use crate::{
    core::client::storage::sss::AWSS3,
    core::client::SQS,
    core::cloud::CloudProvider,
    types::params::{AlertArgs, CronArgs, QueueArgs, StorageArgs},
    OrchestratorError, OrchestratorResult,
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
    /// new_with_gcs - Create a new ResourceFactory with default resource creators for Orchestrator
    /// with GCS Cloud Provider
    pub fn new_with_gcs(
        cloud_provider: Arc<CloudProvider>,
        queue_params: QueueArgs,
        cron_params: CronArgs,
        storage_params: StorageArgs,
        alert_params: AlertArgs,
    ) -> Self {
        let creators = HashMap::new();
        ResourceFactory { creators, cloud_provider, queue_params, cron_params, storage_params, alert_params }
    }

    /// new_with_aws - Create a new ResourceFactory with default resource creators for Orchestrator
    /// with AWS Cloud Provider
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
        // creators.insert(ResourceType::Cron, Box::new(EventBridgeResourceCreator) as Box<dyn ResourceCreator>);
        // creators.insert(ResourceType::Notification, Box::new(SNSResourceCreator) as Box<dyn ResourceCreator>);

        ResourceFactory { creators, cloud_provider, queue_params, cron_params, storage_params, alert_params }
    }

    pub async fn setup_resource(&self) -> OrchestratorResult<()> {
        for (resource_type, creator) in self.creators.iter() {
            info!(" ⏳ Setting up resource: {:?}", resource_type);
            let mut resource = creator.create_resource(self.cloud_provider.clone()).await?;
            match resource_type {
                ResourceType::Storage => {
                    let rs = resource.downcast_mut::<AWSS3>().unwrap();
                    rs.setup(self.storage_params.clone()).await?;
                }
                ResourceType::Queue => {
                    let rs = resource.downcast_mut::<SQS>().unwrap();
                    rs.setup(self.queue_params.clone()).await?;
                }
                ResourceType::Notification => {
                    let rs = resource.downcast_mut::<SNS>().unwrap();
                    rs.setup(self.alert_params.clone()).await?;
                }
                ResourceType::Cron => {
                    let rs = resource.downcast_mut::<EventBridgeClient>().unwrap();
                    rs.setup(self.cron_params.clone()).await?;
                }
                _ => {}
            }
            info!(" ✅ Resource setup completed: {:?}", resource_type);
        }
        Ok(())
    }

    /// Register a new resource creator
    pub fn register(&mut self, resource_type: ResourceType, creator: Box<dyn ResourceCreator>) {
        self.creators.insert(resource_type, creator);
    }

    /// Create a resource of the specified type
    pub async fn create_resource(
        &self,
        resource_type: ResourceType,
        cloud_provider: Arc<CloudProvider>,
    ) -> OrchestratorResult<ResourceWrapper> {
        match self.creators.get(&resource_type) {
            Some(creator) => creator.create_resource(cloud_provider).await,
            None => Err(OrchestratorError::ResourceError(format!(
                "No creator registered for resource type {:?}",
                resource_type
            ))),
        }
    }

    /// Create a resource from a string type
    pub async fn create_resource_from_str(
        &self,
        resource_type: &str,
        cloud_provider: Arc<CloudProvider>,
    ) -> OrchestratorResult<ResourceWrapper> {
        match ResourceType::from_str(resource_type) {
            Some(rt) => self.create_resource(rt, cloud_provider).await,
            None => Err(OrchestratorError::ResourceError(format!("Unknown resource type: {}", resource_type))),
        }
    }
}
