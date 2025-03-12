use crate::core::cloud::CloudProvider;
use crate::resource::aws::s3::{S3Bucket, S3Service};
use crate::resource::aws::sqs::SQS;
use crate::resource::Resource;
use crate::{OrchestratorError, OrchestratorResult};

mod s3;
mod sqs;

#[cfg(test)]
mod tests;

pub trait AWSResource: Resource {
    type SetupResult;
}

pub fn new_aws_resource(cloud_provider: CloudProvider, resource: String) -> OrchestratorResult<Box<dyn Resource>> {
    match resource.as_str() {
        "s3" => Ok(Box::new(S3Service::new(cloud_provider)?)),
        "sqs" => Ok(Box::new(SQS::new(cloud_provider)?)),
        _ => Err(OrchestratorError::UnidentifiedResourceError(resource)),
    }
}
