use crate::{core::client::storage::StorageClient, types::params::StorageArgs};

use crate::core::client::storage::StorageError;
use crate::types::params::AWSResourceIdentifier;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::operation::put_object_tagging::PutObjectTaggingError;
use aws_sdk_s3::types::{Tag, Tagging};
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

fn is_not_found_tagging_error(err: &SdkError<PutObjectTaggingError>) -> bool {
    let service_error = err
        .as_service_error()
        .and_then(|e| e.code())
        .map(|code| matches!(code, "NoSuchKey" | "NotFound"))
        .unwrap_or(false);

    let status_404 = err.raw_response().map(|raw| raw.status().as_u16() == 404).unwrap_or(false);

    service_error || status_404
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

    async fn health_check(&self) -> Result<(), StorageError> {
        // Verify S3 bucket accessibility by checking if it exists
        // This operation requires ListBucket permission
        self.client()
            .head_bucket()
            .bucket(self.bucket_name()?)
            .send()
            .await
            .map_err(|e| StorageError::ObjectStreamError(format!("S3 health check failed: {}", e)))?;

        Ok(())
    }

    /// Tag an object with the specified key-value pairs.
    ///
    /// # Arguments
    /// * `key` - The key of the object to tag.
    /// * `tags` - A slice of (key, value) string slice pairs representing the tags.
    ///
    /// # Returns
    /// * `Result<(), StorageError>` - The result of the tagging operation.
    async fn tag_object<'a>(&self, key: &str, tags: &'a [(&'a str, &'a str)]) -> Result<(), StorageError> {
        let s3_tags: Result<Vec<Tag>, _> = tags.iter().map(|(k, v)| Tag::builder().key(*k).value(*v).build()).collect();

        let s3_tags = s3_tags.map_err(|e| StorageError::ObjectStreamError(format!("Failed to build tags: {:?}", e)))?;

        let tagging = Tagging::builder()
            .set_tag_set(Some(s3_tags))
            .build()
            .map_err(|e| StorageError::ObjectStreamError(format!("Failed to build tagging object: {:?}", e)))?;

        self.client().put_object_tagging().bucket(self.bucket_name()?).key(key).tagging(tagging).send().await.map_err(
            |e| {
                if is_not_found_tagging_error(&e) {
                    StorageError::NotFound(format!("Object '{}' not found", key))
                } else {
                    StorageError::ObjectStreamError(format!("Failed to tag object '{}': {}", key, e))
                }
            },
        )?;

        Ok(())
    }
}
