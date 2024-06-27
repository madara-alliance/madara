use crate::data_storage::aws_s3::config::AWSS3Config;
use crate::data_storage::DataStorage;
use async_trait::async_trait;
use aws_sdk_s3::config::{Builder, Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::{Client, Error};
use bytes::Bytes;

/// Module for AWS S3 config structs and implementations
pub mod config;

/// AWSS3 represents AWS S3 client object containing the client and the config itself.
pub struct AWSS3 {
    client: Client,
    config: AWSS3Config,
}

/// Implementation for AWS S3 client. Contains the function for :
///
/// - initializing a new AWS S3 client
impl AWSS3 {
    /// Initializes a new AWS S3 client by passing the config
    /// and returning it.
    #[allow(dead_code)]
    pub async fn new(config: AWSS3Config) -> Self {
        // AWS cred building
        let credentials = Credentials::new(
            config.s3_key_id.clone(),
            config.s3_key_secret.clone(),
            None,
            None,
            "loaded_from_custom_env",
        );
        let region = Region::new(config.s3_bucket_region.clone().to_string());
        let conf_builder = Builder::new().region(region).credentials_provider(credentials);
        let conf = conf_builder.build();

        // Building AWS S3 config
        let client = Client::from_conf(conf);

        Self { client, config }
    }
}

/// Implementation of `DataStorage` for `AWSS3`
/// contains the function for getting the data and putting the data
/// by taking the key as an argument.
#[async_trait]
impl DataStorage for AWSS3 {
    /// Function to get the data from S3 bucket by Key.
    async fn get_data(&self, key: &str) -> Result<Bytes, Error> {
        let response = self.client.get_object().bucket(self.config.s3_bucket_name.clone()).key(key).send().await?;
        let data_stream = response.body.collect().await.expect("Failed to convert body into AggregatedBytes.");
        let data_bytes = data_stream.into_bytes();
        Ok(data_bytes)
    }

    /// Function to put the data to S3 bucket by Key.
    async fn put_data(&self, data: ByteStream, key: &str) -> Result<(), Error> {
        self.client
            .put_object()
            .bucket(self.config.s3_bucket_name.clone())
            .key(key)
            .body(data)
            .content_type("application/json")
            .send()
            .await?;

        Ok(())
    }
}
