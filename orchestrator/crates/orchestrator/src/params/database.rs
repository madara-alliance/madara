use crate::cli::database::mongodb::MongoDBCliArgs;

/// Validated MongoDB parameters
#[derive(Debug, Clone)]
pub struct MongoConfig {
    pub connection_url: String,
    pub database_name: String,
}

impl From<MongoDBCliArgs> for MongoConfig {
    fn from(args: MongoDBCliArgs) -> Self {
        Self {
            connection_url: args.mongodb_connection_url.unwrap(),
            database_name: args.mongodb_database_name.unwrap(),
        }
    }
}