use crate::env_utils::get_env_var_or_panic;
use crate::settings::{Settings, SettingsProviderError};

#[derive(Debug, Clone, Default)]
pub struct EnvSettingsProvider {}

impl Settings for EnvSettingsProvider {
    fn get_settings(&self, name: &'static str) -> Result<String, SettingsProviderError> {
        Ok(get_env_var_or_panic(name))
    }
}
