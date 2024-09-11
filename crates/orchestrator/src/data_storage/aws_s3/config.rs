use serde::{Deserialize, Serialize};
use utils::settings::Settings;

use crate::data_storage::DataStorageConfig;

/// Represents AWS S3 config struct with all the necessary variables.
#[derive(Clone, Serialize, Deserialize)]
pub struct AWSS3Config {
    /// S3 Bucket Name
    pub bucket_name: String,
}

/// Implementation of `DataStorageConfig` for `AWSS3Config`
impl DataStorageConfig for AWSS3Config {
    /// To return the config struct by creating it from the environment variables.
    fn new_with_settings(settings: &impl Settings) -> Self {
        Self { bucket_name: settings.get_settings_or_panic("AWS_S3_BUCKET_NAME") }
    }
}
