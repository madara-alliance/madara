use crate::cli::Layer;
use crate::core::client::storage::s3::InnerAWSS3;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::params::AWSResourceIdentifier;
use crate::types::params::StorageArgs;
use crate::{OrchestratorError, OrchestratorResult};
use async_trait::async_trait;
use aws_sdk_s3::Error as S3Error;
use std::sync::Arc;
use tracing::{info, warn};

#[async_trait]
impl Resource for InnerAWSS3 {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = S3Error;
    type SetupArgs = StorageArgs;
    type CheckArgs = AWSResourceIdentifier;

    /// create_setup creates a new S3 client and returns an instance of AWSS3
    ///
    /// # Arguments
    /// * `cloud_provider` - The cloud provider configuration.
    ///
    /// # Returns
    /// * `OrchestratorResult<Self>` - The result of the setup operation.
    ///
    async fn create_setup(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config)),
        }
    }
    /// Set up a new S3 bucket
    async fn setup(&self, _layer: Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        let default_region = self.0.config().region().map(|r| r.to_string()).unwrap_or_else(|| "us-east-1".to_string());
        let (bucket_name, region) = match &args.bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                let region = if arn.region.is_empty() {
                    default_region
                } else {
                    arn.region.to_string() // Convert to String to match the other branch
                };
                (arn.resource.clone(), region)
            }
            AWSResourceIdentifier::Name(name) => (name.to_string(), default_region),
        };
        tracing::info!("Bucket Name: {}", bucket_name);

        // it is special to s3 that it can have empty region in it's arn : e.g: arn:aws:s3:::karnot-mo-bucket
        // in such scenarios we would want to default to provided region

        // Check if the bucket already exists
        // If it does, return the existing bucket name and location
        if self.check_if_exists(&args.bucket_identifier).await? {
            warn!(" ⏭️  S3 bucket '{}' already exists", bucket_name);
            return Ok(());
        }

        info!("Creating New Bucket: {}", bucket_name);
        info!("Creating bucket in region: {}", region);

        let mut bucket_builder = self.0.create_bucket().bucket(&bucket_name);

        if region != "us-east-1" {
            let constraint = aws_sdk_s3::types::BucketLocationConstraint::from(region.as_str());
            let cfg = aws_sdk_s3::types::CreateBucketConfiguration::builder().location_constraint(constraint).build();
            bucket_builder = bucket_builder.create_bucket_configuration(cfg);
        }

        let _result = bucket_builder.send().await.map_err(|e| {
            OrchestratorError::ResourceSetupError(format!("Failed to create S3 bucket '{}': {:?}", bucket_name, e))
        })?;
        Ok(())
    }

    // TODO: can we simplify if check_if_exists and is_ready_to_use are same ?
    async fn check_if_exists(&self, bucket_identifier: &Self::CheckArgs) -> OrchestratorResult<bool> {
        let bucket_name = match bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => &arn.resource,
            AWSResourceIdentifier::Name(name) => &name,
        };

        Ok(self.client().head_bucket().bucket(bucket_name).send().await.is_ok())
    }

    async fn is_ready_to_use(&self, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        let bucket_name = match &args.bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => &arn.resource,
            AWSResourceIdentifier::Name(name) => &name,
        };

        Ok(self.client().head_bucket().bucket(bucket_name).send().await.is_ok())
    }
}
