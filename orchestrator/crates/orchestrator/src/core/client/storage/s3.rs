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
    pub(crate) client: Arc<Client>,
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
            let (name, region) = Self::parse_bucket_identifier(&args.bucket_identifier);
            (Some(name), region)
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
    fn parse_bucket_identifier(identifier: &str) -> (String, Option<String>) {
        // Check if the identifier is an ARN
        if identifier.starts_with("arn:aws:s3:") {
            // Parse the ARN to extract region and bucket name
            let parts: Vec<&str> = identifier.split(':').collect();

            if parts.len() >= 6 {
                let region = if !parts[3].is_empty() { Some(parts[3].to_string()) } else { None };

                // Handle different ARN formats
                let bucket_name = if parts[5].contains('/') {
                    // Format: arn:aws:s3:region:account-id:bucket/bucket-name
                    let resource_parts: Vec<&str> = parts[5].split('/').collect();
                    if resource_parts[0] == "bucket" && resource_parts.len() > 1 {
                        resource_parts[1].to_string()
                    } else {
                        // Just use the whole resource part as bucket name
                        parts[5].to_string()
                    }
                } else {
                    // Format: arn:aws:s3:::bucket-name
                    parts[5].to_string()
                };

                return (bucket_name, region);
            }
        }

        // If not an ARN or parsing failed, just use the identifier as the bucket name
        (identifier.to_string(), None)
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
