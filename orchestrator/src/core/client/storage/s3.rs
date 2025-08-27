use crate::{core::client::storage::StorageClient, types::params::StorageArgs};

use crate::core::client::storage::StorageError;
use crate::types::params::AWSResourceIdentifier;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_s3::Client;
use bytes::Bytes;

/// AWSS3 is a struct that represents an AWS S3 client.
#[derive(Clone, Debug)]
pub(crate) struct InnerAWSS3(Client);

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
        Self(client)
    }

    pub fn client(&self) -> &Client {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct AWSS3 {
    inner: InnerAWSS3,
    bucket_identifier: AWSResourceIdentifier,
}

impl AWSS3 {
    /// Creates a new instance of AWSS3 with the provided AWS configuration and optional arguments.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The storage arguments.
    ///
    /// # Returns
    /// * `Self` - The new instance of AWSS3.
    pub fn new(aws_config: &SdkConfig, args: &StorageArgs) -> Self {
        Self { inner: InnerAWSS3::new(aws_config), bucket_identifier: args.bucket_identifier.clone() }
    }

    pub(crate) fn bucket_name(&self) -> Result<String, StorageError> {
        match &self.bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                // For S3 ARNs, the bucket name is in the resource field
                // S3 ARN format: arn:aws:s3:::bucket-name
                Ok(arn.resource.clone())
            }
            AWSResourceIdentifier::Name(name) => Ok(name.clone()),
        }
    }

    pub(crate) fn client(&self) -> &Client {
        self.inner.client()
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
        Ok(self.client().delete_object().bucket(self.bucket_name()?).key(key).send().await.map(|_| ())?)
    }

    async fn list_files_in_dir(&self, dir_path: &str) -> Result<Vec<String>, StorageError> {
        let mut file_paths = Vec::new();

        // Ensure the directory path ends with '/' for proper prefix matching
        let prefix = if dir_path.ends_with('/') { dir_path.to_string() } else { format!("{}/", dir_path) };

        let mut paginator =
            self.client().list_objects_v2().bucket(self.bucket_name()?).prefix(&prefix).into_paginator().send();

        // Iterate through all pages of results
        while let Some(page) = paginator.next().await {
            if let Some(objects) = page
                .map_err(|e| {
                    StorageError::ObjectStreamError(format!("Failed to list files in path {}: {}", dir_path, e))
                })?
                .contents
            {
                for object in objects {
                    if let Some(key) = object.key {
                        // Skip directories (keys ending with '/')
                        if !key.ends_with('/') {
                            file_paths.push(key);
                        }
                    }
                }
            }
        }

        Ok(file_paths)
    }
}
