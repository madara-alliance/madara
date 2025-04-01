use aws_config::meta::region::RegionProviderChain;
use aws_config::{Region, SdkConfig};
use aws_credential_types::Credentials;
use crate::cli::provider::{AWSConfigValidatedArgs, ProviderValidatedArgs};

#[derive(Debug, Clone)]
pub struct AWSCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> SdkConfig {
        let region = self.region.clone();
        let region_provider = RegionProviderChain::first_try(Region::new(region)).or_default_provider();
        let credentials =
            Credentials::from_keys(self.access_key_id.clone(), self.secret_access_key.clone(), None);
        aws_config::from_env().credentials_provider(credentials).region(region_provider).load().await
    }
}

impl From<AWSConfigValidatedArgs> for AWSCredentials {
    fn from(args: AWSConfigValidatedArgs) -> Self {
        Self {
            access_key_id: args.aws_access_key_id,
            secret_access_key: args.aws_secret_access_key,
            region: args.aws_region,
        }
    }
}
