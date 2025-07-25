// =============================================================================
// BOOTSTRAPPER SERVICE - Setup utility for L1/L2 initialization
// =============================================================================

pub mod config;
// Re-export common utilities
pub use config::*;
use crate::services::constants::*;

use crate::services::server::{Server, ServerConfig};
use std::process::ExitStatus;

const CONNECTION_ATTEMPTS: usize = 1;
const CONNECTION_DELAY_MS: u64 = 100;

pub struct BootstrapperService {
    server: Server,
    config: BootstrapperConfig,
}

impl BootstrapperService {
    /// Run the bootstrapper and wait for completion (convenience method)
    pub async fn run(config: BootstrapperConfig) -> Result<ExitStatus, BootstrapperError> {
        let mut service = Self::start(config).await?;
        service.wait_for_completion().await
    }

    /// Start a new bootstrapper service with the given configuration
    pub async fn start(config: BootstrapperConfig) -> Result<Self, BootstrapperError> {
        // Build the bootstrapper command
        let command = config.to_command();

        // Create server config - bootstrapper doesn't need network port,
        // but we'll use a dummy port for the generic server interface
        let server_config = ServerConfig {
            connection_attempts: CONNECTION_ATTEMPTS, // No connection check needed
            connection_delay_ms: CONNECTION_DELAY_MS,
            service_name: format!("Bootstrapper-{}", config.mode().to_string()),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(BootstrapperError::Server)?;

        Ok(Self { server, config })
    }

    /// Wait for the bootstrapper to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<ExitStatus, BootstrapperError> {
        println!("🚀 Running bootstrapper in {} mode...", self.config.mode());

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(self.config.timeout(), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited() {
                    return Ok::<ExitStatus, BootstrapperError>(exit_status);
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
                    let error_msg = if let Some(code) = exit_status.code() {
                            format!("Process exited with code: {}", code)
                        } else {
                            format!("Process terminated by signal: {}", exit_status)
                    };
                    Err(BootstrapperError::SetupFailed(error_msg))
                }
            }
            Ok(Err(e)) => Err(BootstrapperError::ExecutionFailed(e.to_string())),
            Err(_) => Err(BootstrapperError::ExecutionFailed(format!(
                "Bootstrapper timed out after {:?}",
                self.config.timeout()
            ))),
        }
    }

    /// Get the mode that was executed
    pub fn mode(&self) -> &BootstrapperMode {
        self.config.mode()
    }

    /// Get the configuration used
    pub fn config(&self) -> &BootstrapperConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), BootstrapperError> {
        println!("☠️ Stopping Bootstrapper");
        self.server.stop().map_err(|err| BootstrapperError::Server(err))
    }

    /// Get logs
    pub fn logs(&self) -> (bool, bool) {
        self.config.logs()
    }


    /// Check if bootstrapper binary exists (static method for convenience)
    pub fn check_binary() -> Result<(), BootstrapperError> {
        let binary_path = std::path::PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY);
        if binary_path.exists() {
            Ok(())
        } else {
            Err(BootstrapperError::BinaryNotFound(format!("Default binary not found: {}", binary_path.display())))
        }
    }

    // update values in the config file
    pub fn update_config_file(key: &str, value: &str) -> Result<(), BootstrapperError> {
        // Update bootstrapper config
        let mut config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(DEFAULT_BOOTSTRAPPER_CONFIG)
                .map_err(|e| BootstrapperError::ConfigReadWriteError(e))?,
        )
        .map_err(|e| BootstrapperError::ConfigParseError(e))?;

        config[key] = serde_json::Value::String(value.to_string());

        std::fs::write(
            DEFAULT_BOOTSTRAPPER_CONFIG,
            serde_json::to_string_pretty(&config)
                .map_err(|e| BootstrapperError::ConfigParseError(e))?,
        )
        .map_err(|e| BootstrapperError::ConfigReadWriteError(e))?;

        println!("✅ Updated bootstrapper config with {} value: {}", key, value);
        Ok(())
    }
}
