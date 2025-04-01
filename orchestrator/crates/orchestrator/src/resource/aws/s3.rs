use crate::core::cloud::CloudProvider;
use crate::error::{OrchestratorError, OrchestratorResult};
use crate::resource::args::StorageArgs;
use crate::resource::Resource;
use async_trait::async_trait;
use aws_sdk_s3::{Client as S3Client, Client, Error as S3Error};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

#[derive(Clone, Debug)]
pub struct SSS {
    pub client: Client,
    pub region: Option<String>,
    pub bucket_name: Option<String>,
}

pub struct S3BucketSetupArgs {
    pub bucket_name: String,
    pub bucket_location_constraint: String,
}

pub struct S3BucketSetupResult {
    pub name: String,
    pub location: Option<String>,
}

// Method to delete a specific bucket
impl SSS {
    pub async fn delete_bucket(&self, bucket_name: &str) -> OrchestratorResult<()> {
        // First, delete all objects in the bucket
        self.delete_all_objects(bucket_name).await?;

        // Then delete the bucket
        self.client
            .delete_bucket()
            .bucket(bucket_name)
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceError(format!("Failed to delete bucket: {}", e)))?;

        Ok(())
    }

    pub async fn list_bucket_contents(&self, bucket_name: &str) -> OrchestratorResult<Vec<String>> {
        let objects = self
            .client
            .list_objects_v2()
            .bucket(bucket_name)
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceError(e.to_string()))?;

        let mut result = Vec::new();
        if let Some(contents) = objects.contents {
            for obj in contents {
                if let Some(key) = obj.key {
                    result.push(key);
                }
            }
        }

        Ok(result)
    }

    pub async fn delete_all_objects(&self, bucket_name: &str) -> OrchestratorResult<()> {
        let objects = self.list_bucket_contents(bucket_name).await?;

        if objects.is_empty() {
            return Ok(());
        }

        let mut delete_objects = Vec::new();
        for key in objects {
            let object_id = aws_sdk_s3::types::ObjectIdentifier::builder()
                .key(key)
                .build()
                .map_err(|e| OrchestratorError::ResourceError(e.to_string()))?;
            delete_objects.push(object_id);
        }

        let delete = aws_sdk_s3::types::Delete::builder()
            .set_objects(Some(delete_objects))
            .build()
            .map_err(|e| OrchestratorError::ResourceError(e.to_string()))?;

        self.client
            .delete_objects()
            .bucket(bucket_name)
            .delete(delete)
            .send()
            .await
            .map_err(|e| OrchestratorError::AwsError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl Resource for SSS {
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
                Ok(Self { client, region: None, bucket_name: None })
            } // _ => Err(OrchestratorError::InvalidCloudProviderError(
              //     "Mismatch Cloud Provider for S3Bucket resource".to_string(),
              // )),
        }
    }
    /// Setup a new S3 bucket
    #[instrument]
    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        // Check if bucket already exists
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
                    // return Err(OrchestratorError::ResourceAlreadyExistsError(format!(
                    //     "S3 bucket '{}' already exists",
                    //     args.bucket_name
                    // )));
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
        let objects = self
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
        if let Some(bucket_name) = &self.bucket_name {
            self.delete_all_objects(bucket_name).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_args() {
        let args = S3BucketSetupArgs {
            bucket_name: "test-bucket".to_string(),
            bucket_location_constraint: "us-east-1".to_string(),
        };

        assert_eq!(args.bucket_name, "test-bucket");
        assert_eq!(args.bucket_location_constraint, "us-east-1");
    }

    #[test]
    fn test_setup_result() {
        let result = S3BucketSetupResult { name: "test-bucket".to_string(), location: Some("us-east-1".to_string()) };

        assert_eq!(result.name, "test-bucket");
        assert_eq!(result.location, Some("us-east-1".to_string()));
    }
}
