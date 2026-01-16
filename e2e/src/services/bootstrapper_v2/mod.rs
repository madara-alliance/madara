// =============================================================================
// BOOTSTRAPPER V2 SERVICE - Setup utility for base layer and Madara initialization
// =============================================================================

pub mod config;
// Re-export common utilities
use crate::services::constants::*;
pub use config::*;

use crate::services::helpers::get_file_path;
use crate::services::server::{Server, ServerConfig};
use std::process::ExitStatus;

const CONNECTION_ATTEMPTS: usize = 1;
const CONNECTION_DELAY_MS: u64 = 100;

pub struct BootstrapperV2Service {
    server: Server,
    config: BootstrapperV2Config,
}

impl BootstrapperV2Service {
    /// Run the bootstrapper v2 and wait for completion (convenience method)
    pub async fn run(config: BootstrapperV2Config) -> Result<ExitStatus, BootstrapperV2Error> {
        let mut service = Self::start(config).await?;
        service.wait_for_completion().await
    }

    /// Start a new bootstrapper v2 service with the given configuration
    pub async fn start(config: BootstrapperV2Config) -> Result<Self, BootstrapperV2Error> {
        // Build the bootstrapper command
        let command = config.to_command();

        // Create server config - bootstrapper doesn't need network port,
        // but we'll use a dummy port for the generic server interface
        let server_config = ServerConfig {
            connection_attempts: CONNECTION_ATTEMPTS, // No connection check needed
            connection_delay_ms: CONNECTION_DELAY_MS,
            service_name: format!("BootstrapperV2-{}", config.mode()),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(BootstrapperV2Error::Server)?;

        Ok(Self { server, config })
    }

    /// Wait for the bootstrapper v2 to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<ExitStatus, BootstrapperV2Error> {
        println!("🚀 Running bootstrapper v2 in {} mode...", self.config.mode());

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(self.config.timeout(), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited()? {
                    return Ok::<ExitStatus, BootstrapperV2Error>(exit_status);
                }

                // Small delay to avoid busy waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!("✅ Bootstrapper V2 {} completed successfully with {}", self.config.mode(), exit_status);
                    Ok(exit_status)
                } else {
                    let error = if let Some(code) = exit_status.code() {
                        BootstrapperV2Error::SetupFailedWithCode(code)
                    } else {
                        BootstrapperV2Error::SetupFailedWithSignal(exit_status.to_string())
                    };
                    Err(error)
                }
            }
            Ok(Err(e)) => Err(BootstrapperV2Error::ExecutionFailed(e.to_string())),
            Err(_) => Err(BootstrapperV2Error::ExecutionTimedOut(self.config.timeout())),
        }
    }

    /// Get the mode that was executed
    pub fn mode(&self) -> BootstrapperV2Mode {
        self.config.mode()
    }

    /// Get the configuration used
    pub fn config(&self) -> &BootstrapperV2Config {
        &self.config
    }

    /// Stop the bootstrapper v2 service
    pub fn stop(&mut self) -> Result<(), BootstrapperV2Error> {
        println!("☠️ Stopping Bootstrapper V2");
        self.server.stop().map_err(BootstrapperV2Error::Server)?;
        Ok(())
    }

    /// Get logs configuration
    pub fn logs(&self) -> (bool, bool) {
        self.config.logs()
    }

    /// Update values in the config file
    /// Supports dot-notation for nested paths (e.g., "base_layer.core_contract_init_data.verifier")
    pub fn update_config_file(key: &str, value: &str) -> Result<(), BootstrapperV2Error> {
        let bootstrapper_config_file = get_file_path(BOOTSTRAPPER_V2_CONFIG);
        let mut config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(bootstrapper_config_file.clone())
                .map_err(BootstrapperV2Error::ConfigReadWriteError)?,
        )
        .map_err(BootstrapperV2Error::ConfigParseError)?;

        // Handle nested paths using dot notation
        let keys: Vec<&str> = key.split('.').collect();
        let mut current = &mut config;

        // Navigate to the parent of the final key
        for k in &keys[..keys.len() - 1] {
            current = current
                .get_mut(*k)
                .ok_or_else(|| BootstrapperV2Error::InvalidConfig(format!("Key '{}' not found in path '{}'", k, key)))?;
        }

        // Set the final key's value
        if let Some(last_key) = keys.last() {
            current[*last_key] = serde_json::Value::String(value.to_string());
        }

        std::fs::write(
            bootstrapper_config_file,
            serde_json::to_string_pretty(&config).map_err(BootstrapperV2Error::ConfigParseError)?,
        )
        .map_err(BootstrapperV2Error::ConfigReadWriteError)?;

        println!("✅ Updated bootstrapper v2 config with {} value: {}", key, value);
        Ok(())
    }
}
