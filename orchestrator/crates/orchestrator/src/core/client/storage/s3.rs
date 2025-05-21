use crate::{core::client::storage::StorageClient, types::params::StorageArgs};

use crate::core::client::storage::StorageError;
use async_trait::async_trait;
use aws_config::Region;
use aws_config::SdkConfig;
use aws_sdk_s3::Client;
use bytes::Bytes;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct AWSS3 {
    pub client: Arc<Client>,
    // We need to keep these as Options since setup's create_setup fns passes args as None.
    bucket_name: Option<String>,
    region: Option<String>,
}

impl AWSS3 {
    /// Creates a new instance of AWSS3 with the provided AWS configuration and optional arguments.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The storage arguments with bucket_identifier (name or ARN).
    ///
    /// # Returns
    /// * `Self` - The new instance of AWSS3.
    pub fn new(aws_config: &SdkConfig, args: Option<&StorageArgs>) -> Self {
        let (bucket_name, region) = if let Some(args) = args {
            // Parse the bucket identifier to handle both ARN and name
            Self::parse_arn_bucket_identifier(args)
        } else {
            (None, None)
        };

        // Configure the S3 client with the right region if specified in ARN
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);

        // Set region from ARN if available and different from config
        if let Some(region) = &region {
            // Only override region if it was explicitly provided in the ARN
            s3_config_builder = s3_config_builder.region(Region::new(region.clone()));
        }

        s3_config_builder = s3_config_builder
            .use_arn_region(true)  // This ensures ARN-specified regions are used
            .force_path_style(true);

        let client = Client::from_conf(s3_config_builder.build());

        Self { client: Arc::new(client), bucket_name, region }
    }

    /// Parse a bucket identifier (name or ARN) into bucket name and optional region
    fn parse_arn_bucket_identifier(args: &StorageArgs) -> (Option<String>, Option<String>) {
        let identifier = &args.bucket_identifier;

        // Handle standard S3 bucket ARN: arn:aws:s3:::{bucket-name}
        if identifier.starts_with("arn:aws:s3:::") {
            let parts: Vec<&str> = identifier.split(':').collect();

            // Standard S3 ARN has format arn:aws:s3:::{bucket-name}
            // After splitting by ':', we expect parts[0]="arn", parts[1]="aws",
            // parts[2]="s3", parts[3]="", parts[4]="", parts[5]="{bucket-name}"
            // Note: We don't add AWS_PREFIX if we received the whole ARN !
            if parts.len() >= 6 {
                let bucket_path = parts[5];

                // If there's a path separator in the bucket name part,
                // extract just the bucket name (everything before first '/')
                let bucket_name = if bucket_path.contains('/') {
                    let name = bucket_path.split('/').next().unwrap_or(bucket_path);
                    name.to_string()
                } else {
                    bucket_path.to_string()
                };

                return (Some(bucket_name.to_string()), None);
            }
        }

        // If not an ARN, just use as a s3 name with prefix
        (Some(format!("{}_{}", args.aws_prefix, identifier)), None)
    }

    pub(crate) fn bucket_name(&self) -> Result<String, StorageError> {
        self.bucket_name.clone().ok_or_else(|| StorageError::InvalidBucketName("Bucket name is not set".to_string()))
    }

    /// Returns the region extracted from ARN, if available
    pub fn region(&self) -> Option<String> {
        self.region.clone()
    }
}

#[async_trait]
impl StorageClient for AWSS3 {
    /// Get the data from the bucket with the specified key.
    ///
    /// # Arguments
    /// * `key` - The key of the object to retrieve.
    ///
    /// # Returns
    /// * `Result<Bytes, StorageError>` - The result of the get operation.
    async fn get_data(&self, key: &str) -> Result<Bytes, StorageError> {
        let output = self.client.get_object().bucket(self.bucket_name()?).key(key).send().await?;
        let data = output.body.collect().await.map_err(|e| StorageError::ObjectStreamError(e.to_string()))?;

        Ok(data.into_bytes())
    }

    /// Put the data into the bucket with the specified key.
    ///
    /// # Arguments
    /// * `data` - The data to put into the bucket.
    /// * `key` - The key of the object to put.
    /// # Returns
    /// * `Result<(), StorageError>` - The result of the put operation.
    async fn put_data(&self, data: Bytes, key: &str) -> Result<(), StorageError> {
        self.client.put_object().bucket(self.bucket_name()?).key(key).body(data.into()).send().await?;
        Ok(())
    }

    /// delete the data from the bucket with the specified key.
    ///
    /// # Arguments
    /// * `key` - The key of the object to delete.
    /// # Returns
    /// * `Result<(), StorageError>` - The result of the delete operation.
    async fn delete_data(&self, key: &str) -> Result<(), StorageError> {
        Ok(self.client.delete_object().bucket(self.bucket_name()?).key(key).send().await.map(|_| ())?)
    }
}
