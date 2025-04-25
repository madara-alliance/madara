use crate::cli::database::mongodb::MongoDBCliArgs;
use crate::cli::RunCmd;
use crate::OrchestratorError;

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


/// Validated MongoDB parameters
#[derive(Debug, Clone)]
pub struct DatabaseArgs {
    pub connection_uri: String,
    pub database_name: String,
}

impl TryFrom<RunCmd> for DatabaseArgs {
    type Error = OrchestratorError;

    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        Ok(Self {
            connection_uri: run_cmd.mongodb_args.mongodb_connection_url.unwrap(),
            database_name: run_cmd.mongodb_args.mongodb_database_name.unwrap(),
        })
    }
}
