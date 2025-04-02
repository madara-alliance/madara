use crate::core::client::storage::StorageClient;
use crate::params::StorageArgs;
use crate::{OrchestratorError, OrchestratorResult};
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_s3::{
    config::Region,
    error::SdkError,
    operation::{
        create_bucket::CreateBucketError, delete_bucket::DeleteBucketError, delete_object::DeleteObjectError,
        put_object::PutObjectError,
    },
    types::BucketLocationConstraint,
    Client,
};
use bytes::Bytes;
use std::str::FromStr;
use std::sync::Arc;

pub struct SSS {
    client: Arc<Client>,
    bucket: String,
    bucket_location_constraint: BucketLocationConstraint,
}

impl SSS {
    pub async fn setup(s3_config: &StorageArgs, aws_config: &SdkConfig) -> OrchestratorResult<Self> {
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
        s3_config_builder.set_force_path_style(Some(true));
        let client = Client::from_conf(s3_config_builder.build());

        let region_str = aws_config
            .region()
            .ok_or(OrchestratorError::InvalidCloudProviderError("AWS region is not set".to_string()))?;
        let bucket_location_constraint = BucketLocationConstraint::from_str(region_str.as_ref())
            .map_err(|_| OrchestratorError::InvalidRegionError)?;

        Ok(Self { client: Arc::new(client), bucket: s3_config.bucket_name.clone(), bucket_location_constraint })
    }
}

#[async_trait]
impl StorageClient for SSS {
    async fn get_data(&self, key: String) -> OrchestratorResult<Bytes> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| OrchestratorError::AWSS3Error(e))?;

        let data = output.body.collect().await.map_err(|e| OrchestratorError::AWSS3StreamError(e.to_string()))?;

        Ok(data.into_bytes())
    }

    async fn put_data(&self, data: Bytes, key: String) -> OrchestratorResult<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(data.into())
            .send()
            .await
            .map_err(|e: SdkError<PutObjectError>| OrchestratorError::AwsError(e.to_string()))?;

        Ok(())
    }

    async fn create_bucket(&self, bucket: String) -> OrchestratorResult<()> {
        let config = aws_sdk_s3::types::CreateBucketConfiguration::builder()
            .location_constraint(self.bucket_location_constraint.clone())
            .build();

        self.client
            .create_bucket()
            .bucket(&bucket)
            .create_bucket_configuration(config)
            .send()
            .await
            .map_err(|e: SdkError<CreateBucketError>| OrchestratorError::AwsError(e.to_string()))?;

        Ok(())
    }

    async fn delete_bucket(&self, bucket: String) -> OrchestratorResult<()> {
        self.client
            .delete_bucket()
            .bucket(&bucket)
            .send()
            .await
            .map_err(|e: SdkError<DeleteBucketError>| OrchestratorError::AwsError(e.to_string()))?;

        Ok(())
    }

    async fn delete_data(&self, key: String) -> OrchestratorResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e: SdkError<DeleteObjectError>| OrchestratorError::AwsError(e.to_string()))?;

        Ok(())
    }
}
