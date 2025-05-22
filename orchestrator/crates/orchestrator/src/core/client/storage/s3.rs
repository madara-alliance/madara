use crate::{core::client::storage::StorageClient, types::params::StorageArgs};

use crate::core::client::storage::StorageError;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_s3::Client;
use bytes::Bytes;
use std::sync::Arc;


/// AWSS3 is a struct that represents an AWS S3 client.
#[derive(Clone, Debug)]
pub(crate) struct InnerAWSS3(pub(crate) Arc<Client>);

impl InnerAWSS3 {
    /// Creates a new instance of InnerAWSS3 with the provided AWS configuration.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    ///
    /// # Returns
    /// * `Self` - The new instance of InnerAWSS3.
    pub fn new(aws_config: &SdkConfig) -> Self {
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
        s3_config_builder.set_force_path_style(Some(true));
        let client = Client::from_conf(s3_config_builder.build());
        Self(Arc::new(client))
    }
}

#[derive(Clone, Debug)]
pub struct AWSS3 {
    pub(crate) inner: InnerAWSS3,
    bucket_name: Option<String>,
}

impl AWSS3 {
    /// Creates a new instance of AWSS3 with the provided AWS configuration and optional arguments.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The storage arguments.
    ///
    /// # Returns
    /// * `Self` - The new instance of AWSS3.
    pub fn new(aws_config: &SdkConfig, args: Option<&StorageArgs>) -> Self {
        Self { inner: InnerAWSS3::new(aws_config), bucket_name: args.map(|a| a.bucket_name.clone()) }
    }

    pub(crate) fn bucket_name(&self) -> Result<String, StorageError> {
        self.bucket_name.clone().ok_or_else(|| StorageError::InvalidBucketName("Bucket name is not set".to_string()))
    }
    
    pub(crate) fn client(&self) -> &Client {
        self.inner.0.as_ref()
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
        let output = self.client().get_object().bucket(self.bucket_name()?).key(key).send().await?;

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
        self.client().put_object().bucket(self.bucket_name()?).key(key).body(data.into()).send().await?;
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
        Ok(self.client().delete_object().bucket(self.bucket_name()?).key(key).send().await.map(|_| ())?)
    }
}
