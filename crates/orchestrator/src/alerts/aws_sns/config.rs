use serde::{Deserialize, Serialize};
use utils::settings::Settings;

#[derive(Clone, Serialize, Deserialize)]
pub struct AWSSNSConfig {
    /// AWS SNS ARN
    pub sns_arn: String,
    /// AWS SNS region
    pub sns_arn_region: String,
}

impl AWSSNSConfig {
    pub fn new_with_settings(settings: &impl Settings) -> color_eyre::Result<Self> {
        Ok(Self {
            sns_arn: settings.get_settings("AWS_SNS_ARN")?,
            sns_arn_region: settings.get_settings("AWS_SNS_REGION")?,
        })
    }
}
