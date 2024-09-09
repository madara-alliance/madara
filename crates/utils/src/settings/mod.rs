pub mod env;

#[derive(Debug, thiserror::Error)]
pub enum SettingsProviderError {
    #[error("Internal settings error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub trait Settings {
    fn get_settings(&self, name: &'static str) -> Result<String, SettingsProviderError>;
}
