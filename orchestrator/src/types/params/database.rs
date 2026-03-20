use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_utils::env_utils::resolve_secret_from_file;

/// Validated MongoDB parameters
#[derive(Debug, Clone)]
pub struct DatabaseArgs {
    pub connection_uri: String,
    pub database_name: String,
}

impl TryFrom<RunCmd> for DatabaseArgs {
    type Error = OrchestratorError;

    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        // Resolve secret: _FILE env var takes precedence over direct value
        let connection_uri = resolve_secret_from_file("MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL")
            .map_err(OrchestratorError::SetupCommandError)?
            .or(run_cmd.mongodb_args.mongodb_connection_url)
            .ok_or(OrchestratorError::SetupCommandError("Database Connection URL is required".to_string()))?;

        Ok(Self {
            connection_uri,
            database_name: run_cmd
                .mongodb_args
                .mongodb_database_name
                .ok_or(OrchestratorError::SetupCommandError("Database Name is required".to_string()))?,
        })
    }
}
