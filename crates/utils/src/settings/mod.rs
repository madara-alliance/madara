pub mod env;

#[derive(Debug, thiserror::Error)]
pub enum SettingsProviderError {
    #[error("Internal settings error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub trait Settings: Send {
    fn get_settings_or_panic(&self, name: &'static str) -> String;
}
