use aws_config::Region;
use aws_config::SdkConfig;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_credential_types::Credentials;
use orchestrator::core::cloud::CloudProvider;

/// Create a mock AWS cloud provider for testing
pub fn mock_aws_provider() -> CloudProvider {
    // Create credentials
    let credentials = Credentials::from_keys("mock_key", "mock_secret", None);
    let creds_provider = SharedCredentialsProvider::new(credentials);

    // Build a basic config with minimal settings
    let config = SdkConfig::builder().region(Region::new("us-east-1")).credentials_provider(creds_provider).build();

    CloudProvider::AWS(Box::new(config))
}
