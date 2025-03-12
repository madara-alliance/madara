#[cfg(test)]
mod s3_tests {
    use crate::core::cloud::CloudProvider;
    use crate::resource::aws::s3::{S3Bucket, S3BucketSetupArgs};
    use crate::resource::Resource;
    use aws_config::meta::region::RegionProviderChain;
    use aws_config::SdkConfig;
    use aws_sdk_s3::config::Region;
    use std::sync::Arc;

    // Basic tests for S3Bucket creation
    #[tokio::test]
    async fn test_new_with_aws_provider() {
        let region = Region::new("us-east-1");
        let config = SdkConfig::builder().region(region).build();
        let cloud_provider = CloudProvider::AWS(Box::new(config));

        let result = S3Bucket::new(cloud_provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_new_with_non_aws_provider() {
        // Create a non-AWS provider (using GCP as an example)
        let cloud_provider = CloudProvider::GCP(Box::new(()));

        let result = S3Bucket::new(cloud_provider).await;
        assert!(result.is_err());
        match result {
            Err(e) => {
                let error_msg = format!("{}", e);
                assert!(error_msg.contains("Mismatch Cloud Provider"));
            }
            _ => panic!("Expected an error"),
        }
    }

    // Integration tests that would connect to real AWS services
    // These are commented out as they require credentials and would create real resources
    // They would be run with a feature flag like `cargo test --features integration-tests`
    /*
    #[cfg(feature = "integration-tests")]
    mod integration {
        use super::*;
        use uuid::Uuid;

        #[tokio::test]
        async fn test_s3_bucket_lifecycle() {
            // Set up AWS config from environment
            let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
            let config = aws_config::from_env().region(region_provider).load().await;
            let cloud_provider = CloudProvider::AWS(Box::new(config));

            // Create S3Bucket
            let s3_bucket = S3Bucket::new(cloud_provider).await.unwrap();

            // Create a unique bucket name for testing
            let bucket_name = format!("test-bucket-{}", Uuid::new_v4().to_string().replace("-", ""));
            let args = S3BucketSetupArgs {
                bucket_name: bucket_name.clone(),
                bucket_location_constraint: "us-east-1".to_string(),
            };

            // 1. Setup - Create bucket
            let setup_result = s3_bucket.setup(args).await;
            assert!(setup_result.is_ok());

            // 2. Check - Verify bucket exists
            let check_result = s3_bucket.check(bucket_name.clone()).await;
            assert!(check_result.is_ok());
            assert_eq!(check_result.unwrap(), true);

            // 3. Delete - Clean up bucket
            let delete_result = s3_bucket.delete_bucket(&bucket_name).await;
            assert!(delete_result.is_ok());

            // 4. Verify - Confirm bucket is gone
            let verify_result = s3_bucket.check(bucket_name).await;
            assert!(verify_result.is_ok());
            assert_eq!(verify_result.unwrap(), false);
        }
    }
    */
}
