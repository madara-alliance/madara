use crate::core::client::storage::s3::InnerAWSS3;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
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

/// Tag key used to mark objects for expiration
const EXPIRATION_TAG_KEY: &str = "expire-after-settlement";
const EXPIRATION_TAG_VALUE: &str = "true";

/// Number of days after object creation before S3 deletes the object
const EXPIRATION_DAYS: i32 = 14;

/// Lifecycle rule ID
const LIFECYCLE_RULE_ID: &str = "expire-settled-artifacts";

#[async_trait]
impl Resource for InnerAWSS3 {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = S3Error;
    type SetupArgs = StorageArgs;
    type CheckArgs = AWSResourceIdentifier;

    /// create_setup creates a new S3 client and returns an instance of AWSS3
    ///
    /// # Arguments
    /// * `cloud_provider` - The cloud provider configuration.
    ///
    /// # Returns
    /// * `OrchestratorResult<Self>` - The result of the setup operation.
    ///
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
        self.setup_lifecycle_rule(&bucket_name).await?;

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
    /// Sets up the lifecycle rule for automatic object expiration based on tags.
    ///
    /// Objects tagged with `expire-after-settlement=true` will be automatically
    /// deleted by S3 after `EXPIRATION_DAYS` days from their creation date.
    ///
    /// This operation is idempotent - it will overwrite any existing lifecycle
    /// configuration with the same rule ID.
    async fn setup_lifecycle_rule(&self, bucket_name: &str) -> OrchestratorResult<()> {
        info!(
            "Setting up S3 lifecycle rule '{}' for bucket '{}' (expire tagged objects after {} days)",
            LIFECYCLE_RULE_ID, bucket_name, EXPIRATION_DAYS
        );

        // Create the tag filter
        let tag_filter = Tag::builder()
            .key(EXPIRATION_TAG_KEY)
            .value(EXPIRATION_TAG_VALUE)
            .build()
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to build tag filter: {:?}", e)))?;

        // Create the lifecycle rule filter (filter by tag)
        let filter = LifecycleRuleFilter::builder().tag(tag_filter).build();

        // Create the expiration action
        let expiration = LifecycleExpiration::builder().days(EXPIRATION_DAYS).build();

        // Create the lifecycle rule
        let rule = LifecycleRule::builder()
            .id(LIFECYCLE_RULE_ID)
            .filter(filter)
            .status(ExpirationStatus::Enabled)
            .expiration(expiration)
            .build()
            .map_err(|e| OrchestratorError::ResourceSetupError(format!("Failed to build lifecycle rule: {:?}", e)))?;

        // Create the lifecycle configuration
        let lifecycle_config = BucketLifecycleConfiguration::builder().rules(rule).build().map_err(|e| {
            OrchestratorError::ResourceSetupError(format!("Failed to build lifecycle configuration: {:?}", e))
        })?;

        // Apply the lifecycle configuration to the bucket
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

        info!(
            "Successfully configured S3 lifecycle rule: objects tagged with '{}={}' will expire after {} days",
            EXPIRATION_TAG_KEY, EXPIRATION_TAG_VALUE, EXPIRATION_DAYS
        );

        Ok(())
    }
}
