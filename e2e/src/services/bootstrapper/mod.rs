// =============================================================================
// LEGACY BOOTSTRAPPER V1 SERVICE - Kept for archive/reference only
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

pub struct LegacyBootstrapperService {
    server: Server,
    config: LegacyBootstrapperConfig,
}

impl LegacyBootstrapperService {
    /// Run the legacy bootstrapper and wait for completion (convenience method)
    pub async fn run(config: LegacyBootstrapperConfig) -> Result<ExitStatus, LegacyBootstrapperError> {
        let mut service = Self::start(config).await?;
        service.wait_for_completion().await
    }

    /// Start the legacy bootstrapper with the given configuration
    pub async fn start(config: LegacyBootstrapperConfig) -> Result<Self, LegacyBootstrapperError> {
        // Build the legacy bootstrapper command
        let command = config.to_command();

        // Create server config - the legacy bootstrapper doesn't need a network port,
        // but we'll use a dummy port for the generic server interface
        let server_config = ServerConfig {
            connection_attempts: CONNECTION_ATTEMPTS, // No connection check needed
            connection_delay_ms: CONNECTION_DELAY_MS,
            service_name: format!("BootstrapperLegacy-{}", config.mode()),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(LegacyBootstrapperError::Server)?;

        Ok(Self { server, config })
    }

    /// Wait for the legacy bootstrapper to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<ExitStatus, LegacyBootstrapperError> {
        println!("🚀 Running legacy bootstrapper in {} mode...", self.config.mode());

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(self.config.timeout(), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited()? {
                    return Ok::<ExitStatus, LegacyBootstrapperError>(exit_status);
                }

                // Small delay to avoid busy waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!(
                        "✅ Legacy bootstrapper {} completed successfully with {}",
                        self.config.mode(),
                        exit_status
                    );
                    Ok(exit_status)
                } else {
                    let error = if let Some(code) = exit_status.code() {
                        LegacyBootstrapperError::SetupFailedWithCode(code)
                    } else {
                        LegacyBootstrapperError::SetupFailedWithSignal(exit_status.to_string())
                    };
                    Err(error)
                }
            }
            Ok(Err(e)) => Err(LegacyBootstrapperError::ExecutionFailed(e.to_string())),
            Err(_) => Err(LegacyBootstrapperError::ExecutionTimedOut(self.config.timeout())),
        }
    }

    /// Get the mode that was executed
    pub fn mode(&self) -> &LegacyBootstrapperMode {
        self.config.mode()
    }

    /// Get the configuration used
    pub fn config(&self) -> &LegacyBootstrapperConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), LegacyBootstrapperError> {
        println!("☠️ Stopping legacy bootstrapper");
        self.server.stop().map_err(LegacyBootstrapperError::Server)?;
        Ok(())
    }

    /// Get logs
    pub fn logs(&self) -> (bool, bool) {
        self.config.logs()
    }

    // update values in the legacy bootstrapper config file
    pub fn update_config_file(key: &str, value: &str) -> Result<(), LegacyBootstrapperError> {
        let bootstrapper_config_file = get_file_path(LEGACY_BOOTSTRAPPER_CONFIG);
        let mut config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(bootstrapper_config_file.clone())
                .map_err(LegacyBootstrapperError::ConfigReadWriteError)?,
        )
        .map_err(LegacyBootstrapperError::ConfigParseError)?;

        config[key] = serde_json::Value::String(value.to_string());

        std::fs::write(
            bootstrapper_config_file,
            serde_json::to_string_pretty(&config).map_err(LegacyBootstrapperError::ConfigParseError)?,
        )
        .map_err(LegacyBootstrapperError::ConfigReadWriteError)?;

        println!("✅ Updated legacy bootstrapper config with {} value: {}", key, value);
        Ok(())
    }
}
