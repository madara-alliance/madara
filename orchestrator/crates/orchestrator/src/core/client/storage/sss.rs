use crate::core::client::StorageClient;
use crate::data_storage::aws_s3::AWSS3ValidatedArgs;
use crate::params::StorageArgs;
use crate::{OrchestratorError, OrchestratorResult};
use aws_config::SdkConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{BucketLocationConstraint, CreateBucketConfiguration};
use aws_sdk_s3::Client;
use bytes::Bytes;
use color_eyre::eyre::Context;
use std::str::FromStr;


pub struct SSS {
    client: Client,
    bucket: String,
    bucket_location_constraint: BucketLocationConstraint,
}

impl SSS {
    pub async fn setup(s3_config: &StorageArgs, aws_config: &SdkConfig) -> OrchestratorResult<Self> {
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
        s3_config_builder.set_force_path_style(Some(true));
        let client = Client::from_conf(s3_config_builder.build());

        let region_str = aws_config.region().unwrap_or_else(|| "us-east-1".into());
        let bucket_location_constraint = BucketLocationConstraint::from_str(region_str.as_str())?;
        Ok(Self { client, bucket: s3_config.bucket_name.clone(), bucket_location_constraint })
    }
}

impl StorageClient for SSS {
    async fn get_data(&self, key: &str) -> OrchestratorResult<Bytes> {
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;

        let data_stream = response.body.collect().await.map_err(|e| OrchestratorError::AWSS3StreamError(e.to_string()))?;

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
    async fn put_data(&self, data: Bytes, key: &str) -> color_eyre::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type("application/json")
            .send()
            .await
            .context(format!("Failed to put object in bucket: {}, key: {}", self.bucket, key))?;

        tracing::debug!(
            log_type = "DataStorage",
            category = "data_storage_call",
            "Successfully put data into {}. key={}",
            self.bucket,
            key
        );
        Ok(())
    }

    async fn create_bucket(&self, bucket_name: &str) -> color_eyre::Result<()> {
        if self.bucket_location_constraint.as_str() == "us-east-1" {
            self.client
                .create_bucket()
                .bucket(bucket_name)
                .send()
                .await
                .context(format!("Failed to create bucket: {} in us-east-1", bucket_name))?;
            return Ok(());
        }

        let bucket_configuration = Some(
            CreateBucketConfiguration::builder().location_constraint(self.bucket_location_constraint.clone()).build(),
        );

        self.client
            .create_bucket()
            .bucket(bucket_name)
            .set_create_bucket_configuration(bucket_configuration)
            .send()
            .await
            .context(format!(
                "Failed to create bucket: {} in region: {}",
                bucket_name,
                self.bucket_location_constraint.as_str()
            ))?;

        Ok(())
    }
    async fn delete_bucket(&self, bucket: &str) -> OrchestratorResult<()> {
        Ok(())
    }
}