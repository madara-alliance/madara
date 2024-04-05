use crate::database::DatabaseConfig;
use crate::utils::env_utils::get_env_var_or_panic;

pub struct MongoDbConfig {
    pub url: String,
}

impl DatabaseConfig for MongoDbConfig {
    fn new_from_env() -> Self {
        Self { url: get_env_var_or_panic("MONGODB_CONNECTION_STRING") }
    }
}
