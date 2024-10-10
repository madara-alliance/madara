use serde::{Deserialize, Serialize};
use utils::settings::Settings;

use crate::database::DatabaseConfig;

#[derive(Debug, Serialize, Deserialize)]
pub struct MongoDbConfig {
    pub url: String,
    pub database_name: String,
}

impl DatabaseConfig for MongoDbConfig {
    fn new_with_settings(settings: &impl Settings) -> Self {
        Self {
            url: settings.get_settings_or_panic("MONGODB_CONNECTION_STRING"),
            database_name: settings.get_settings_or_panic("DATABASE_NAME"),
        }
    }
}
