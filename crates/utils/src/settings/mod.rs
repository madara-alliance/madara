pub mod default;

use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum SettingsProviderError {
    #[error("Internal settings error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub trait SettingsProvider {
    fn get_settings<T: DeserializeOwned + Default>(&self, name: &'static str) -> Result<T, SettingsProviderError>;
}
