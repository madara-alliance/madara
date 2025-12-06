use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(rename = "type")]
    pub db_type: DatabaseType,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mongodb: Option<MongoDBConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    MongoDB,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDBConfig {
    pub connection_url: String,

    #[serde(default = "default_db_name")]
    pub database_name: String,
}

fn default_db_name() -> String {
    "orchestrator".to_string()
}
