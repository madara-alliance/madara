use crate::{core::client::storage::StorageClient, types::params::StorageArgs};

use crate::core::client::storage::StorageError;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_s3::Client;
use bytes::Bytes;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct AWSS3 {
    pub(crate) client: Arc<Client>,
    bucket_name: Option<String>,
    #[allow(dead_code)]
    region: Option<String>,
}

impl AWSS3 {
    /// Constructor is used form the setup of the Resource
    /// Creates a new instance of AWSS3 with the provided client and optional bucket name and region.
    /// used for testing purposes
    /// # Arguments
    /// * `client` - The S3 client.
    /// * `bucket_name` - The name of the S3 bucket.
    /// * `region` - The region of the S3 bucket.
    ///
    /// # Returns
    /// * `Self` - The new instance of AWSS3.
    pub(crate) fn constructor(client: Arc<Client>, bucket_name: Option<String>, region: Option<String>) -> Self {
        Self { client, bucket_name, region }
    }
    /// Creates a new instance of AWSS3 with the provided configuration.
    ///
    /// # Arguments
    /// * `s3_config` - The configuration for the S3 client.
    /// * `aws_config` - The AWS SDK configuration.
    ///
    /// # Returns
    /// * `Result<Self, StorageError>` - The result of the creation operation.
    pub async fn create(s3_config: &StorageArgs, aws_config: &SdkConfig) -> Result<Self, StorageError> {
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
        s3_config_builder.set_force_path_style(Some(true));
        let client = Client::from_conf(s3_config_builder.build());

        Ok(Self {
            client: Arc::new(client),
            bucket_name: Some(s3_config.bucket_name.clone()),
            region: s3_config.bucket_location_constraint.clone(),
        })
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
        // Note: unwrap is safe here because the bucket name is set in the constructor
        let output = self.client.get_object().bucket(self.bucket_name.clone().unwrap()).key(key).send().await?;

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
        // Note: unwrap is safe here because the bucket name is set in the constructor
        self.client.put_object().bucket(self.bucket_name.clone().unwrap()).key(key).body(data.into()).send().await?;
        Ok(())
    }

    /// delete the data from the bucket with the specified key.
    ///
    /// # Arguments
    /// * `key` - The key of the object to delete.
    /// # Returns
    /// * `Result<(), StorageError>` - The result of the delete operation.
    async fn delete_data(&self, key: &str) -> Result<(), StorageError> {
        // Note: unwrap is safe here because the bucket name is set in the constructor
        let result = self.client.delete_object().bucket(self.bucket_name.clone().unwrap()).key(key).send().await;
        match result {
            Ok(_) => Ok(()),
            Err(err) => Err(StorageError::DeleteObjectError(err)),
        }
    }
}
