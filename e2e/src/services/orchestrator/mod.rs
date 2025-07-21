// =============================================================================
// ORCHESTRATOR SERVICE - Using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use std::process::ExitStatus;

use std::time::Duration;

pub struct OrchestratorService {
    server: Server, // None for setup mode
    config: OrchestratorConfig,
}

impl OrchestratorService {

    pub async fn run(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        // TODO: config mode should only be run

        let command = config.to_command();

        println!("Running orchestrator in run mode with command : {:?}", command);

        let port = config.port().unwrap();

        // Create server config
        let server_config = ServerConfig {
            rpc_port: Some(port),
            service_name: format!("Orchestrator-{}", config.mode().to_string()),
            connection_attempts: 60, // Orchestrator might take time to start
            connection_delay_ms: 2000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(OrchestratorError::Server)?;

        Ok(Self { server, config })
    }

    pub async fn setup(config: OrchestratorConfig) -> Result<ExitStatus, OrchestratorError> {
        // TODO: config mode should only be setup

        let mut service = Self::start_setup_mode(config).await?;
        service.wait_for_completion().await
    }


    /// Wait for the bootstrapper to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<ExitStatus, OrchestratorError> {
        println!("🚀 Running orchestrator in {} mode...", self.config.mode());

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(Duration::from_secs(360), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited() {
                    return Ok::<ExitStatus, OrchestratorError>(exit_status);
                }

                // Small delay to avoid busy waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!("✅ Orchestrator {} completed successfully with {}", self.config.mode(), exit_status);
                    Ok(exit_status)
                } else {
                    let exit_code = exit_status.code().unwrap_or(-1);
                    Err(OrchestratorError::SetupFailed(exit_code))
                }
            }
            Ok(Err(e)) => Err(OrchestratorError::ExecutionFailed(e.to_string())),
            Err(_) => Err(OrchestratorError::ExecutionFailed(format!(
                "Orchestrator timed out after {:?}",
                Duration::from_secs(360)
            ))),
        }
    }


    /// Run in setup mode (blocking, returns when complete)
    async fn start_setup_mode(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        let command = config.to_command();

        println!("Running orchestrator in setup mode with command : {:?}", command);

        let server_config = ServerConfig {
            service_name: format!("Orchestrator-{}", config.mode().to_string()),
            ..Default::default()

        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(OrchestratorError::Server)?;

        Ok(Self { server, config })
    }


    // TODO: these dependencies should be typed and not a Vec<String>

    /// Get the dependencies required by the orchestrator
    pub fn dependencies(&self) -> Vec<String> {
        vec![
            // internal
            "anvil".to_string(),
            "madara".to_string(),
            "pathfinder".to_string(),
            // TODO: Actually bootstrapper is not a direct dep of orchestrator
            // we can remove this
            "bootstrapper_l1".to_string(),
            "bootstrapper_l2".to_string(),
            // external
            "atlantic".to_string(),
            "localstack".to_string(),
            "mongodb".to_string(),
        ]
    }

    // TODO: Will need a endpoint() fn here

    // TODO: A mongodb respective fn that dumps and loads the db

    /// Get the current mode
    pub fn mode(&self) -> &OrchestratorMode {
        self.config.mode()
    }

    /// Get the port number (run mode only)
    pub fn port(&self) -> Option<u16> {
        self.config.port()
    }

    /// Get the layer
    pub fn layer(&self) -> &Layer {
        self.config.layer()
    }

    /// Get the configuration used
    pub fn config(&self) -> &OrchestratorConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), OrchestratorError> {
        println!("☠️ Stopping Orchestrator");
        self.server.stop().map_err(|err| OrchestratorError::Server(err))
    }

    /// Get the underlying server (run mode only)
    pub fn server(&self) -> &Server {
        &self.server
    }
}
