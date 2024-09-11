use crate::env_utils::get_env_var_or_panic;
use crate::settings::Settings;

#[derive(Debug, Clone, Default)]
pub struct EnvSettingsProvider {}

impl Settings for EnvSettingsProvider {
    fn get_settings_or_panic(&self, name: &'static str) -> String {
        get_env_var_or_panic(name)
    }
}
