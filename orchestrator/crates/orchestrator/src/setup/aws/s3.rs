use crate::core::client::storage::sss::{S3BucketSetupResult, AWSS3};
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::params::StorageArgs;
use crate::{OrchestratorError, OrchestratorResult};
use async_trait::async_trait;
use aws_sdk_s3::{Client as S3Client, Error as S3Error};
use std::sync::Arc;
use tracing::{info, warn};

#[async_trait]
impl Resource for AWSS3 {
    type SetupResult = S3BucketSetupResult;
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = S3Error;
    type SetupArgs = StorageArgs;
    type CheckArgs = String; // Bucket name

    async fn new(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => {
                let client = S3Client::new(&aws_config);
                Ok(Self::constructor(Arc::new(client), None, None))
            }
            _ => Err(OrchestratorError::InvalidCloudProviderError(
                "Mismatch Cloud Provider for S3Bucket resource".to_string(),
            ))?,
        }
    }
    /// Set up a new S3 bucket
    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        // REVIEW: 23 : Why are we not using the .check function to check for the bucket already existing.
        let existing_buckets = &self
            .client
            .list_buckets()
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to list buckets: {}", e)))?;

        for bucket in existing_buckets.buckets() {
            if let Some(name) = &bucket.name {
                if name == &args.bucket_name {
                    warn!("S3 bucket '{}' already exists", args.bucket_name);
                    return Ok(S3BucketSetupResult { name: args.bucket_name, location: None });
                }
            }
        }
        info!("Creating New Bucket: {}", args.bucket_name);

        // Get the current region from the client config
        let region = self.client.config().region().map(|r| r.to_string()).unwrap_or_else(|| "us-east-1".to_string());
        info!("Creating bucket in region: {}", region);

        let mut bucket = self.client.create_bucket().bucket(&args.bucket_name);

        if region != "us-east-1" {
            let constraint = aws_sdk_s3::types::BucketLocationConstraint::from(region.as_str());
            let cfg = aws_sdk_s3::types::CreateBucketConfiguration::builder().location_constraint(constraint).build();
            bucket = bucket.create_bucket_configuration(cfg);
        }

        let result = bucket.send().await.map_err(|e| {
            OrchestratorError::ResourceSetupError(format!("Failed to create S3 bucket '{}': {:?}", args.bucket_name, e))
        })?;
        Ok(S3BucketSetupResult { name: args.bucket_name, location: result.location })
    }

    async fn check(&self, bucket_name: Self::CheckArgs) -> OrchestratorResult<Self::CheckResult> {
        self
            .client
            .list_objects_v2()
            .bucket(&bucket_name)
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceError(e.to_string()))?;

        // Just check if we can list objects
        Ok(true)
    }

    async fn teardown(&self) -> OrchestratorResult<()> {
        if let Some(bucket_name) = &self.bucket_name() {
            // self.delete_all_objects(bucket_name).await?;

            self.client
                .delete_bucket()
                .bucket(bucket_name)
                .send()
                .await
                .map_err(|e| OrchestratorError::ResourceError(format!("Failed to delete bucket: {}", e)))?;

            Ok(())
        } else {
            Err(OrchestratorError::ResourceError("Bucket name is not set".to_string()))
        }
    }
}
