use crate::core::cloud::CloudProvider;
use crate::error::{OrchestratorError, OrchestratorResult};
use crate::resource::aws::s3::SSS;
use crate::resource::aws::sqs::SQS;
use crate::resource::Resource;
use async_trait::async_trait;
use std::any::Any;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ResourceType {
    S3,
    SQS,
}

impl ResourceType {
    pub fn from_str(resource_type: &str) -> Option<Self> {
        match resource_type.to_lowercase().as_str() {
            "s3" => Some(ResourceType::S3),
            "sqs" => Some(ResourceType::SQS),
            _ => None,
        }
    }
}

// ResourceWrapper to type-erase the specific resource types
pub struct ResourceWrapper {
    resource: Box<dyn Any + Send + Sync>,
    resource_type: ResourceType,
}

impl ResourceWrapper {
    pub fn new<R>(resource: R, resource_type: ResourceType) -> Self
    where
        R: Any + Send + Sync,
    {
        ResourceWrapper { resource: Box::new(resource), resource_type }
    }

    pub fn get_type(&self) -> &ResourceType {
        &self.resource_type
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.resource.downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.resource.downcast_mut::<T>()
    }
}

/// A trait for resource creation strategies to enable flexible resource instantiation
#[async_trait]
pub trait ResourceCreator: Send + Sync {
    async fn create_resource(&self, cloud_provider: CloudProvider) -> OrchestratorResult<ResourceWrapper>;
}

// S3 resource creator
pub struct S3ResourceCreator;

#[async_trait]
impl ResourceCreator for S3ResourceCreator {
    async fn create_resource(&self, cloud_provider: CloudProvider) -> OrchestratorResult<ResourceWrapper> {
        let s3 = SSS::new(cloud_provider).await?;
        Ok(ResourceWrapper::new(s3, ResourceType::S3))
    }
}

// SQS resource creator
pub struct SQSResourceCreator;

#[async_trait]
impl ResourceCreator for SQSResourceCreator {
    async fn create_resource(&self, cloud_provider: CloudProvider) -> OrchestratorResult<ResourceWrapper> {
        let sqs = SQS::new(cloud_provider).await?;
        Ok(ResourceWrapper::new(sqs, ResourceType::SQS))
    }
}

/// ResourceFactory is responsible for creating resources based on their type
pub struct ResourceFactory {
    creators: HashMap<ResourceType, Box<dyn ResourceCreator>>,
}

impl ResourceFactory {
    /// Create a new ResourceFactory with default resource creators
    pub fn new() -> Self {
        let mut creators = HashMap::new();
        creators.insert(ResourceType::S3, Box::new(S3ResourceCreator) as Box<dyn ResourceCreator>);
        creators.insert(ResourceType::SQS, Box::new(SQSResourceCreator) as Box<dyn ResourceCreator>);

        ResourceFactory { creators }
    }

    /// Register a new resource creator
    pub fn register(&mut self, resource_type: ResourceType, creator: Box<dyn ResourceCreator>) {
        self.creators.insert(resource_type, creator);
    }

    /// Create a resource of the specified type
    pub async fn create_resource(
        &self,
        resource_type: ResourceType,
        cloud_provider: CloudProvider,
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
        cloud_provider: CloudProvider,
    ) -> OrchestratorResult<ResourceWrapper> {
        match ResourceType::from_str(resource_type) {
            Some(rt) => self.create_resource(rt, cloud_provider).await,
            None => Err(OrchestratorError::ResourceError(format!("Unknown resource type: {}", resource_type))),
        }
    }
}

/// Setup function that initializes all necessary resources
pub async fn setup(resources: Vec<(&str, CloudProvider)>) -> OrchestratorResult<Vec<ResourceWrapper>> {
    let factory = ResourceFactory::new();
    let mut setup_resources = Vec::new();

    for (resource_type, cloud_provider) in resources {
        let resource = factory.create_resource_from_str(resource_type, cloud_provider).await?;
        setup_resources.push(resource);
    }

    Ok(setup_resources)
}

/// Set up a single resource
pub async fn setup_resource(resource_type: &str, cloud_provider: CloudProvider) -> OrchestratorResult<ResourceWrapper> {
    let factory = ResourceFactory::new();
    factory.create_resource_from_str(resource_type, cloud_provider).await
}
