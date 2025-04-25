use crate::core::client::event_bus::event_bridge::EventBridgeClient;
use crate::core::client::SNS;
use crate::{
    core::client::{queue::sqs::SQS, storage::s3::AWSS3},
    core::cloud::CloudProvider,
    core::traits::resource::Resource,
    setup::wrapper::ResourceWrapper,
    OrchestratorResult,
};
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ResourceType {
    Queue,
    Storage,
    EventBus,
    PubSub,
}

/// A trait for resource creation strategies to enable flexible resource instantiation
#[async_trait]
pub trait ResourceCreator: Send + Sync {
    async fn create_resource_client(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper>;
}

// S3 resource creator
pub struct S3ResourceCreator;

#[async_trait]
impl ResourceCreator for S3ResourceCreator {
    async fn create_resource_client(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper> {
        let s3 = AWSS3::create_setup(cloud_provider.clone()).await?;
        Ok(ResourceWrapper::new(cloud_provider, s3, ResourceType::Storage))
    }
}

// SQS resource creator
pub struct SQSResourceCreator;

#[async_trait]
impl ResourceCreator for SQSResourceCreator {
    async fn create_resource_client(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper> {
        let sqs = SQS::create_setup(cloud_provider.clone()).await?;
        Ok(ResourceWrapper::new(cloud_provider, sqs, ResourceType::Queue))
    }
}

// SNS resource creator
pub struct SNSResourceCreator;

#[async_trait]
impl ResourceCreator for SNSResourceCreator {
    async fn create_resource_client(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper> {
        let sns = SNS::create_setup(cloud_provider.clone()).await?;
        Ok(ResourceWrapper::new(cloud_provider, sns, ResourceType::PubSub))
    }
}

// SNS resource creator
pub struct EventBridgeResourceCreator;

#[async_trait]
impl ResourceCreator for EventBridgeResourceCreator {
    async fn create_resource_client(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper> {
        let eb_client = EventBridgeClient::create_setup(cloud_provider.clone()).await?;
        Ok(ResourceWrapper::new(cloud_provider, eb_client, ResourceType::EventBus))
    }
}
