// =============================================================================
// ORCHESTRATOR SERVICE - Using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::servers::server::{Server, ServerConfig};
use std::process::ExitStatus;

use std::time::Duration;
use tokio::process::Command;

pub struct OrchestratorService {
    server: Server, // None for setup mode
    config: OrchestratorConfig,
}

impl OrchestratorService {

    pub async fn start(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        // TODO: config mode should only be run

        let command = config.to_command();

        println!("Running orchestrator in run mode with command : {:?}", command);

        let port = config.port().unwrap();
        let host = format!("127.0.0.1:{}", port);

        // Create server config
        let server_config = ServerConfig {
            port,
            host,
            connection_attempts: 60, // Orchestrator might take time to start
            connection_delay_ms: 2000,
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
        println!("🚀 Running bootstrapper in {} mode...", self.config.mode());

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
                    println!("✅ Bootstrapper {} completed successfully with {}", self.config.mode(), exit_status);
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
            skip_wait_for_ready: true,
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(OrchestratorError::Server)?;

        // // For setup mode, we run the command directly and wait for completion
        // let mut child = command.spawn().map_err(|e| OrchestratorError::Server(ServerError::StartupFailed(e)))?;

        // // Wait for the process to complete
        // let status = child.wait().await.map_err(|e| OrchestratorError::Server(ServerError::Io(e)))?;

        // if status.success() {
        //     println!("Orchestrator cloud setup completed ✅");
        //     Ok(Self { server: None, config, address: None })
        // } else {
        //     let exit_code = status.code().unwrap_or(-1);
        //     Err(OrchestratorError::SetupFailed(exit_code))
        // }
        Ok(Self { server, config })
    }


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

    /// Validate that all required dependencies are available and running
    /// TODO: might move this to a fn in setup
    pub async fn validate_dependencies(&self) -> Result<(), OrchestratorError> {
        // TODO: complete this!
        let dependencies = self.dependencies();

        for dep in dependencies {
            // For now, just check if the command exists
            // You might want to implement more sophisticated checking
            let result = Command::new(&dep).arg("--version").output().await;

            if result.is_err() {
                return Err(OrchestratorError::MissingDependency(dep));
            }
        }

        Ok(())
    }


    /// Get the endpoint URL for the orchestrator service (run mode only)
    // pub fn endpoint(&self) -> Option<Url> {
    //     // TODO: validate run mode is being used
    //     if let Some(ref address) = self. {
    //         Url::parse(&format!("http://{}", address)).ok()
    //     } else {
    //         None
    //     }
    // }

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

    /// Get the underlying server (run mode only)
    pub fn server(&self) -> &Server {
        &self.server
    }
}
