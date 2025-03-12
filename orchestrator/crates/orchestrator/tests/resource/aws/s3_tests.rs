use orchestrator::resource::aws::s3::{S3BucketSetupArgs, S3BucketSetupResult};

#[test]
fn test_setup_args() {
    let args = S3BucketSetupArgs {
        bucket_name: "test-bucket".to_string(),
        bucket_location_constraint: "us-east-1".to_string(),
    };

    assert_eq!(args.bucket_name, "test-bucket");
    assert_eq!(args.bucket_location_constraint, "us-east-1");
}

#[test]
fn test_setup_result() {
    let result = S3BucketSetupResult { name: "test-bucket".to_string(), location: Some("us-east-1".to_string()) };

    assert_eq!(result.name, "test-bucket");
    assert_eq!(result.location, Some("us-east-1".to_string()));
}
