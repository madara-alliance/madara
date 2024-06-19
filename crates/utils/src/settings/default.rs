use super::SettingsProvider;

#[derive(Debug, Clone, Default)]
pub struct DefaultSettingsProvider {}

impl SettingsProvider for DefaultSettingsProvider {
    fn get_settings<T: serde::de::DeserializeOwned + Default>(
        &self,
        _section: &'static str,
    ) -> Result<T, super::SettingsProviderError> {
        Ok(T::default())
    }
}
