use crate::cli::SetupCmd;
use crate::OrchestratorError;

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

/// TDOD move this try implementation out of cli code
impl TryFrom<SetupCmd> for AWSConfigValidatedArgs {
    type Error = OrchestratorError;
    fn try_from(setup_cmd: SetupCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            aws_access_key_id: setup_cmd.aws_config_args.aws_access_key_id.clone(),
            aws_secret_access_key: setup_cmd.aws_config_args.aws_secret_access_key.clone(),
            aws_region: setup_cmd.aws_config_args.aws_region.clone(),
        })
    }
}
