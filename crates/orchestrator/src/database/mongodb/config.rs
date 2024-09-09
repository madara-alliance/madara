use serde::{Deserialize, Serialize};
use utils::settings::Settings;

use crate::database::DatabaseConfig;

#[derive(Debug, Serialize, Deserialize)]
pub struct MongoDbConfig {
    pub url: String,
}

impl DatabaseConfig for MongoDbConfig {
    fn new_with_settings(settings: &impl Settings) -> Self {
        Self {
            url: settings
                .get_settings("MONGODB_CONNECTION_STRING")
                .expect("Not able to get MONGODB_CONNECTION_STRING form the given settings"),
        }
    }
}
