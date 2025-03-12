use crate::core::cloud::CloudProvider;
use crate::error::OrchestratorResult;
use crate::resource::setup::{ResourceFactory, ResourceWrapper};
use crate::resource::Resource;

pub mod s3;
pub mod sqs;

pub trait AWSResource: Resource {
    type SetupResult;
}

pub async fn new_aws_resource(cloud_provider: CloudProvider, resource: String) -> OrchestratorResult<ResourceWrapper> {
    // Create a resource factory
    let factory = ResourceFactory::new();

    // Use the factory to create the resource
    factory.create_resource_from_str(&resource, cloud_provider).await
}
