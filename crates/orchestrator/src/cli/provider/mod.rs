pub mod aws;

#[derive(Debug, Clone)]
pub enum ProviderValidatedArgs {
    AWS(AWSConfigValidatedArgs),
}

#[derive(Debug, Clone)]
pub struct AWSConfigValidatedArgs {
    pub aws_access_key_id: String,
    pub aws_secret_access_key: String,
    pub aws_region: String,
}
