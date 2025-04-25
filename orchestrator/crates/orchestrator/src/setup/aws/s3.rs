use crate::core::client::storage::s3::{S3BucketSetupResult, AWSS3};
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
    type CheckArgs = String;

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
            CloudProvider::AWS(aws_config) => {
                let client = S3Client::new(aws_config);
                Ok(Self::constructor(Arc::new(client), None, None))
            }
        }
    }
    /// Set up a new S3 bucket
    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {

        // Check if the bucket already exists
        // If it does, return the existing bucket name and location
        if self.check_if_exists(args.bucket_name.clone()).await? {
            warn!(" ℹ️  S3 bucket '{}' already exists", args.bucket_name);
            return Ok(S3BucketSetupResult { name: args.bucket_name, location: None });
        }

        let existing_buckets = &self
            .client
            .list_buckets()
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to list buckets: {}", e)))?;

        for bucket in existing_buckets.buckets() {
            if let Some(name) = &bucket.name {
                if name == &args.bucket_name {
                    warn!(" ℹ️  S3 bucket '{}' already exists", args.bucket_name);
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

    async fn check_if_exists(&self, bucket_name: Self::CheckArgs) -> OrchestratorResult<bool> {
        Ok(self.client.head_bucket().bucket(bucket_name).send().await.is_ok())
    }

    async fn is_ready_to_use(&self, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        let client = self.client.clone();
        let bucket_name = args.bucket_name.clone();
        let result = client.head_bucket().bucket(bucket_name).send().await;
        Ok(result.is_ok())
    }
}
