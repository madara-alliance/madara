use crate::cli::provider::aws::AWSConfigCliArgs;
use aws_config::meta::region::RegionProviderChain;
use aws_config::{Region, SdkConfig};

#[derive(Debug, Clone)]
pub struct AWSCredentials {
    pub region: String,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> SdkConfig {
        let region = self.region.clone();
        let region_provider = RegionProviderChain::first_try(Region::new(region)).or_default_provider();
        aws_config::from_env().region(region_provider).load().await
    }
}

impl From<AWSConfigCliArgs> for AWSCredentials {
    fn from(args: AWSConfigCliArgs) -> Self {
        Self { region: args.aws_region }
    }
}
