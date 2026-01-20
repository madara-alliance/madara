use crate::core::client::storage::s3::InnerAWSS3;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::constant::{STORAGE_EXPIRATION_TAG_KEY, STORAGE_EXPIRATION_TAG_VALUE, STORAGE_LIFECYCLE_RULE_ID};
use crate::types::params::AWSResourceIdentifier;
use crate::types::params::StorageArgs;
use crate::types::Layer;
use crate::{OrchestratorError, OrchestratorResult};
use async_trait::async_trait;
use aws_sdk_s3::types::{
    BucketLifecycleConfiguration, ExpirationStatus, LifecycleExpiration, LifecycleRule, LifecycleRuleFilter, Tag,
};
use aws_sdk_s3::Error as S3Error;
use std::sync::Arc;
use tracing::{info, warn};

#[async_trait]
impl Resource for InnerAWSS3 {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = S3Error;
    type SetupArgs = StorageArgs;
    type CheckArgs = AWSResourceIdentifier;

    async fn create_setup(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config)),
        }
    }
    /// Set up a new S3 bucket
    async fn setup(&self, _layer: &Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        let bucket_name = match &args.bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => arn.resource.clone(),
            AWSResourceIdentifier::Name(name) => name.clone(),
        };

        // Check if the bucket already exists
        // If it does, skip creation but still ensure lifecycle rule is set
        if self.check_if_exists(&args.bucket_identifier).await? {
            warn!("  S3 bucket {} already exists , skipping creation", &args.bucket_identifier);
        } else {
            // s3 can have empty region in it's arn : e.g: arn:aws:s3:::mo-bucket
            // in such scenarios we would want to default to provided region

            match &args.bucket_identifier {
                AWSResourceIdentifier::ARN(arn) => {
                    // If ARN is provided in setup, we have already checked if it exists
                    tracing::info!("Bucket Arn provided, skipping setup for {}", &arn.resource);
                }
                AWSResourceIdentifier::Name(bucket_name) => {
                    let region = self
                        .client()
                        .config()
                        .region()
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "us-east-1".to_string());

                    info!("Creating New Bucket: {}", bucket_name);

                    let mut bucket_builder = self.client().create_bucket().bucket(bucket_name);

                    if region != "us-east-1" {
                        let constraint = aws_sdk_s3::types::BucketLocationConstraint::from(region.as_str());
                        let cfg = aws_sdk_s3::types::CreateBucketConfiguration::builder()
                            .location_constraint(constraint)
                            .build();
                        bucket_builder = bucket_builder.create_bucket_configuration(cfg);
                    }

                    let _result = bucket_builder.send().await.map_err(|e| {
                        OrchestratorError::ResourceSetupError(format!(
                            "Failed to create S3 bucket '{}': {:?}",
                            bucket_name, e
                        ))
                    })?;
                }
            }
        }

        // Setup lifecycle rule for automatic expiration (idempotent - safe to run on existing buckets)
        self.setup_lifecycle_rule(&bucket_name, args.storage_expiration_days).await?;

        Ok(())
    }

    // TODO: can we simplify if check_if_exists and is_ready_to_use are same ?
    async fn check_if_exists(&self, bucket_identifier: &Self::CheckArgs) -> OrchestratorResult<bool> {
        let bucket_name = match bucket_identifier {
            AWSResourceIdentifier::ARN(arn) => &arn.resource,
            AWSResourceIdentifier::Name(name) => name,
        };

        Ok(self.client().head_bucket().bucket(bucket_name).send().await.is_ok())
    }

    async fn is_ready_to_use(&self, _layer: &Layer, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        Ok(self.check_if_exists(&args.bucket_identifier).await?)
    }
}

impl InnerAWSS3 {
    /// Sets up lifecycle rule for auto-expiration of tagged objects.
    /// Note: put_bucket_lifecycle_configuration replaces entire config, not just this rule.
    /// IMP: If adding more lifecycle rules, fetch existing config with
    /// `get_bucket_lifecycle_configuration` and merge rules before applying.
    async fn setup_lifecycle_rule(&self, bucket_name: &str, expiration_days: i32) -> OrchestratorResult<()> {
        info!("Setting up S3 lifecycle rule '{}' for bucket '{}'", STORAGE_LIFECYCLE_RULE_ID, bucket_name);

        let tag_filter = Tag::builder()
            .key(STORAGE_EXPIRATION_TAG_KEY)
            .value(STORAGE_EXPIRATION_TAG_VALUE)
            .build()
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to build tag filter: {:?}", e)))?;

        let rule = LifecycleRule::builder()
            .id(STORAGE_LIFECYCLE_RULE_ID)
            .filter(LifecycleRuleFilter::builder().tag(tag_filter).build())
            .status(ExpirationStatus::Enabled)
            .expiration(LifecycleExpiration::builder().days(expiration_days).build())
            .build()
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to build lifecycle rule: {:?}", e)))?;

        let lifecycle_config = BucketLifecycleConfiguration::builder().rules(rule).build().map_err(|e| {
            OrchestratorError::ResourceSetupError(format!("Failed to build lifecycle configuration: {:?}", e))
        })?;

        self.client()
            .put_bucket_lifecycle_configuration()
            .bucket(bucket_name)
            .lifecycle_configuration(lifecycle_config)
            .send()
            .await
            .map_err(|e| {
                OrchestratorError::ResourceSetupError(format!(
                    "Failed to set lifecycle configuration on bucket '{}': {:?}",
                    bucket_name, e
                ))
            })?;

        info!("Configured lifecycle rule: tagged objects expire after {} days", expiration_days);
        Ok(())
    }
}
