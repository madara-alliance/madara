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
    async fn setup(&self, _layer: &Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        // Check if the bucket already exists
        // If it does, return the existing bucket name and location
        if self.check_if_exists(&args.bucket_identifier).await? {
            warn!(" ⏭️  S3 bucket {} already exists , skipping creation", &args.bucket_identifier);
            return Ok(());
        }

        // s3 can have empty region in it's arn : e.g: arn:aws:s3:::mo-bucket
        // in such scenarios we would want to default to provided region

        match &args.bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                // If ARN is provided in setup, we have already checked if it exists
                tracing::info!("Bucket Arn provided, skipping setup for {}", &arn.resource);
                Ok(())
            }
            AWSResourceIdentifier::Name(bucket_name) => {
                let region =
                    self.client().config().region().map(|r| r.to_string()).unwrap_or_else(|| "us-east-1".to_string());

                info!("Creating New Bucket: {}", bucket_name);

                let mut bucket_builder = self.client().create_bucket().bucket(bucket_name);

                if region != "us-east-1" {
                    let constraint = aws_sdk_s3::types::BucketLocationConstraint::from(region.as_str());
                    let cfg =
                        aws_sdk_s3::types::CreateBucketConfiguration::builder().location_constraint(constraint).build();
                    bucket_builder = bucket_builder.create_bucket_configuration(cfg);
                }

                let _result = bucket_builder.send().await.map_err(|e| {
                    OrchestratorError::ResourceSetupError(format!(
                        "Failed to create S3 bucket '{}': {:?}",
                        bucket_name, e
                    ))
                })?;
                Ok(())
            }
        }
    }

    // TODO: can we simplify if check_if_exists and is_ready_to_use are same ?
    async fn check_if_exists(&self, bucket_identifier: &Self::CheckArgs) -> OrchestratorResult<bool> {
        let bucket_name = match bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => &arn.resource,
            AWSResourceIdentifier::Name(name) => name,
        };

        Ok(self.client().head_bucket().bucket(bucket_name).send().await.is_ok())
    }

    async fn is_ready_to_use(&self, _layer: &Layer, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        Ok(self.check_if_exists(&args.bucket_identifier).await?)
    }
}
