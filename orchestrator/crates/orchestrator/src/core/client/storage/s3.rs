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
    pub(crate) fn constructor(client: Arc<Client>, bucket_name: Option<String>, region: Option<String>) -> Self {
        Self { client, bucket_name, region }
    }
    pub(crate) fn bucket_name(&self) -> Option<String> {
        self.bucket_name.clone()
    }
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
    async fn get_data(&self, key: &str) -> Result<Bytes, StorageError> {
        let output = self.client.get_object().bucket(self.bucket_name.clone().unwrap()).key(key).send().await?;

        let data = output.body.collect().await.map_err(|e| StorageError::ObjectStreamError(e.to_string()))?;

        Ok(data.into_bytes())
    }

    async fn put_data(&self, data: Bytes, key: &str) -> Result<(), StorageError> {
        self.client.put_object().bucket(self.bucket_name.clone().unwrap()).key(key).body(data.into()).send().await?;

        Ok(())
    }

    async fn delete_data(&self, key: &str) -> Result<(), StorageError> {
        let result = self.client.delete_object().bucket(self.bucket_name.clone().unwrap()).key(key).send().await;
        match result {
            Ok(_) => Ok(()),
            Err(err) => Err(StorageError::DeleteObjectError(err)),
        }
    }
}
