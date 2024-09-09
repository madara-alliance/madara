use std::str::FromStr;

use url::Url;
use utils::env_utils::get_env_var_or_panic;

#[allow(dead_code)]
pub struct MongoDbServer {
    endpoint: Url,
}

impl MongoDbServer {
    pub async fn run() -> Self {
        Self { endpoint: Url::from_str(&get_env_var_or_panic("MONGODB_CONNECTION_STRING")).unwrap() }
    }

    pub fn endpoint(&self) -> Url {
        Url::from_str(&get_env_var_or_panic("MONGODB_CONNECTION_STRING")).unwrap()
    }
}
