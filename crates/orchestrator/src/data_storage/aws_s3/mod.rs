use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use bytes::Bytes;
use color_eyre::Result;
use utils::settings::Settings;

use crate::config::ProviderConfig;
use crate::data_storage::aws_s3::config::AWSS3Config;
use crate::data_storage::{DataStorage, DataStorageConfig};

pub const S3_SETTINGS_NAME: &str = "s3";

/// Module for AWS S3 config structs and implementations
pub mod config;

/// AWSS3 represents AWS S3 client object containing the client and the config itself.
pub struct AWSS3 {
    client: Client,
    bucket: String,
}

/// Implementation for AWS S3 client. Contains the function for :
///
/// - initializing a new AWS S3 client
impl AWSS3 {
    /// To init the struct with main settings
    pub async fn new_with_settings(settings: &impl Settings, provider_config: Arc<ProviderConfig>) -> Self {
        let s3_config = AWSS3Config::new_with_settings(settings);
        let aws_config = provider_config.get_aws_client_or_panic();
        // Building AWS S3 config
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
        // this is necessary for it to work with localstack in test cases
        s3_config_builder.set_force_path_style(Some(true));
        let client = Client::from_conf(s3_config_builder.build());
        Self { client, bucket: s3_config.bucket_name }
    }
}

/// Implementation of `DataStorage` for `AWSS3`
/// contains the function for getting the data and putting the data
/// by taking the key as an argument.
#[async_trait]
impl DataStorage for AWSS3 {
    /// Function to get the data from S3 bucket by Key.
    async fn get_data(&self, key: &str) -> Result<Bytes> {
        let response = self.client.get_object().bucket(&self.bucket).key(key).send().await?;
        let data_stream = response.body.collect().await.expect("Failed to convert body into AggregatedBytes.");
        tracing::debug!("DataStorage: Collected response body into data stream from {}, key={}", self.bucket, key);
        let data_bytes = data_stream.into_bytes();
        tracing::debug!(
            log_type = "DataStorage",
            category = "data_storage_call",
            data_bytes = data_bytes.len(),
            "Successfully retrieved and converted data from {}, key={}",
            self.bucket,
            key
        );
        Ok(data_bytes)
    }

    /// Function to put the data to S3 bucket by Key.
    async fn put_data(&self, data: Bytes, key: &str) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type("application/json")
            .send()
            .await?;

        tracing::debug!(
            log_type = "DataStorage",
            category = "data_storage_call",
            "Successfully put data into {}. key={}",
            self.bucket,
            key
        );
        Ok(())
    }

    async fn build_test_bucket(&self, bucket_name: &str) -> Result<()> {
        self.client.create_bucket().bucket(bucket_name).send().await?;
        Ok(())
    }
}
