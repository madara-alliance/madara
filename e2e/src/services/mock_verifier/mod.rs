// =============================================================================
// L2 : MOCK VERIFIER DEPLOYER SERVICE - Deployment utility for mock GPS verifier contract
// =============================================================================

pub mod config;
// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use std::process::ExitStatus;

// TODO: make this use address and not string
type VerifierAddress = String;

pub struct MockVerifierDeployerService {
    server: Server,
    config: MockVerifierDeployerConfig,
}

// I don't need to return the ExitStatus on completion, I can also return the verifier address

impl MockVerifierDeployerService {
    /// Run the mock verifier deployment and wait for completion (convenience method)
    pub async fn run(config: MockVerifierDeployerConfig) -> Result<VerifierAddress, MockVerifierDeployerError> {
        let mut service = Self::start(config).await?;
        service.wait_for_completion().await
    }

    /// Start a new mock verifier service with the given configuration
    pub async fn start(config: MockVerifierDeployerConfig) -> Result<Self, MockVerifierDeployerError> {
        // Build the deployment script command
        let command = config.to_command();

        // Create server config - mock verifier doesn't need network port,
        // but we'll use the generic server interface
        let server_config = ServerConfig {
            connection_attempts: 1, // No connection check needed
            connection_delay_ms: 100,
            logs: config.logs(),
            service_name: "MockVerifierDeployer".to_string(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(MockVerifierDeployerError::Server)?;

        Ok(Self { server, config })
    }

    /// Wait for the mock verifier deployment to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<VerifierAddress, MockVerifierDeployerError> {
        println!("🚀 Deploying mock verifier contract...");

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(self.config.timeout(), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited() {
                    return Ok::<ExitStatus, MockVerifierDeployerError>(exit_status);
                }

                // Small delay to avoid busy waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!("✅ Mock verifier deployed successfully with {}", exit_status);
                    Ok(self.get_verifier_address_from_output_file()?)
                } else {
                    let exit_code = exit_status.code().unwrap_or(-1);
                    Err(MockVerifierDeployerError::DeploymentFailed(exit_code))
                }
            }
            Ok(Err(e)) => Err(MockVerifierDeployerError::ExecutionFailed(e.to_string())),
            Err(_) => Err(MockVerifierDeployerError::ExecutionFailed(format!(
                "Mock verifier deployment timed out after {:?}",
                self.config.timeout()
            ))),
        }
    }

    /// Get access to the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Get the configuration used
    pub fn config(&self) -> &MockVerifierDeployerConfig {
        &self.config
    }

    /// Get the deployed verifier address from the output file
    pub fn get_verifier_address_from_output_file(&self) -> Result<String, MockVerifierDeployerError> {
        std::fs::read_to_string(&self.config.verifier_file_name())
            .map(|s| s.trim().to_string())
            .map_err(|e| MockVerifierDeployerError::FileSystem(e))
    }

    /// Get dependencies
    pub fn dependencies(&self) -> Option<Vec<String>> {
        Some(vec![
            "anvil".to_string(),
            "forge".to_string(),
        ])
    }

    /// Check if deployment script exists (static method for convenience)
    pub fn check_script() -> Result<(), MockVerifierDeployerError> {
        let script_path = std::path::PathBuf::from(DEFAULT_SCRIPT_PATH);
        if script_path.exists() {
            Ok(())
        } else {
            Err(MockVerifierDeployerError::ScriptNotFound(format!(
                "Default script not found: {}",
                script_path.display()
            )))
        }
    }

    /// Check if deployment script exists for a specific path
    pub fn check_script_path(path: &std::path::Path) -> Result<(), MockVerifierDeployerError> {
        if path.exists() {
            Ok(())
        } else {
            Err(MockVerifierDeployerError::ScriptNotFound(format!(
                "Script not found: {}",
                path.display()
            )))
        }
    }
}
