use crate::cli::RunCmd;
use crate::OrchestratorError;

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
            connection_uri: run_cmd
                .mongodb_args
                .mongodb_connection_url
                .ok_or(OrchestratorError::SetupCommandError("Database Connection URL is required".to_string()))?,
            database_name: run_cmd
                .mongodb_args
                .mongodb_database_name
                .ok_or(OrchestratorError::SetupCommandError("Database Name is required".to_string()))?,
        })
    }
}
