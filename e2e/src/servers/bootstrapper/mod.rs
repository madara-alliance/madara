// =============================================================================
// BOOTSTRAPPER SERVICE - Setup utility for L1/L2 initialization using generic Server
// =============================================================================

pub mod config;
// Re-export common utilities
pub use config::*;

use crate::servers::server::{Server, ServerConfig};
use tokio::process::Command;
use std::process::ExitStatus;

pub struct BootstrapperService {
    server: Server,
    config: BootstrapperConfig,
}

impl BootstrapperService {
    /// Start a new bootstrapper service with the given configuration
    pub async fn start(config: BootstrapperConfig) -> Result<Self, BootstrapperError> {
        // Validate that bootstrapper binary exists
        if !Self::check_bootstrapper_installed(&config).await {
            return Err(BootstrapperError::BinaryNotFound(
                config.binary_path().display().to_string()
            ));
        }

        // Build the bootstrapper command
        let command = config.to_command();

        // Create server config - bootstrapper doesn't need network port,
        // but we'll use a dummy port for the generic server interface
        let server_config = ServerConfig {
            port: 0, // Dummy port - bootstrapper doesn't serve on a port
            host: "127.0.0.1".to_string(),
            skip_wait_for_ready: true,
            connection_attempts: 1, // No connection check needed
            connection_delay_ms: 100,
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(BootstrapperError::Server)?;

        Ok(Self { server, config })
    }

    /// Check if bootstrapper binary is installed/exists
    async fn check_bootstrapper_installed(config: &BootstrapperConfig) -> bool {
        config.binary_path().exists() &&
        Command::new(config.binary_path())
            .arg("--help")
            .output()
            .await
            .is_ok()
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
                    let exit_code = exit_status.code().unwrap_or(-1);
                    Err(BootstrapperError::SetupFailed(exit_code))
                }
            }
            Ok(Err(e)) => Err(BootstrapperError::ExecutionFailed(e.to_string())),
            Err(_) => Err(BootstrapperError::ExecutionFailed(format!(
                "Bootstrapper timed out after {:?}",
                self.config.timeout()
            ))),
        }
    }

    /// Run the bootstrapper and wait for completion (convenience method)
    pub async fn run(config: BootstrapperConfig) -> Result<ExitStatus, BootstrapperError> {
        let mut service = Self::start(config).await?;
        service.wait_for_completion().await
    }

    /// Get access to the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Get the mode that was executed
    pub fn mode(&self) -> &BootstrapperMode {
        self.config.mode()
    }

    /// Get the configuration used
    pub fn config(&self) -> &BootstrapperConfig {
        &self.config
    }

    /// Get dependencies
    pub fn dependencies(&self) -> Option<Vec<String>> {
        Some(if *self.config().mode() == BootstrapperMode::SetupL1 {
            vec!["anvil".to_string()]
        } else {
            vec!["anvil".to_string(), "madara".to_string(), "bootstrapper_l1".to_string()]
        })
    }

    /// Check if bootstrapper binary exists (static method for convenience)
    pub fn check_binary() -> Result<(), BootstrapperError> {
        let binary_path = std::path::PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY);
        if binary_path.exists() {
            Ok(())
        } else {
            Err(BootstrapperError::BinaryNotFound(format!(
                "Default binary not found: {}",
                binary_path.display()
            )))
        }
    }
}


// // =============================================================================
// // BOOTSTRAPPER SERVICE - Setup utility for L1/L2 initialization
// // =============================================================================

// pub mod config;
// // Re-export common utilities
// pub use config::*;

// use std::path::PathBuf;
// use std::process::ExitStatus;

// pub struct BootstrapperService {
//     config: BootstrapperConfig,
// }

// impl BootstrapperService {
//     /// Create a new bootstrapper service with the given configuration
//     pub fn new(config: BootstrapperConfig) -> Result<Self, BootstrapperError> {
//         Ok(Self { config })
//     }

//     /// Execute the bootstrapper command
//     pub async fn run(&self) -> Result<ExitStatus, BootstrapperError> {
//         println!("🚀 Running bootstrapper in {} mode...", self.config.mode());

//         let mut command = self.config.to_command();
//         println!("Bootstrapper Command : {:?}", command);


//         // Use timeout to prevent hanging
//         let result = tokio::time::timeout(self.config.timeout(), async {
//             // Spawn the process
//             let mut child = command
//                 .spawn()
//                 .map_err(|e| BootstrapperError::ExecutionFailed(e.to_string()))?;

//             // Wait for completion
//             let exit_status = child
//                 .wait()
//                 .map_err(|e| BootstrapperError::ExecutionFailed(e.to_string()))?;

//             Ok::<ExitStatus, BootstrapperError>(exit_status)
//         })
//         .await;

//         match result {
//             Ok(Ok(exit_status)) => {
//                 if exit_status.success() {
//                     println!("✅ Bootstrapper {} completed successfully", self.config.mode());
//                     Ok(exit_status)
//                 } else {
//                     let exit_code = exit_status.code().unwrap_or(-1);
//                     Err(BootstrapperError::SetupFailed(exit_code))
//                 }
//             }
//             Ok(Err(e)) => Err(e),
//             Err(_) => Err(BootstrapperError::ExecutionFailed(format!(
//                 "Bootstrapper timed out after {:?}",
//                 self.config.timeout()
//             ))),
//         }
//     }

//     /// Check if bootstrapper binary exists
//     pub fn check_binary() -> Result<(), BootstrapperError> {
//         let binary_path = PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY);
//         if binary_path.exists() {
//             Ok(())
//         } else {
//             Err(BootstrapperError::BinaryNotFound(format!(
//                 "Default binary not found: {}",
//                 binary_path.display()
//             )))
//         }
//     }

//     /// Get the mode that was executed
//     pub fn mode(&self) -> &BootstrapperMode {
//         self.config.mode()
//     }

//     /// Get the configuration used
//     pub fn config(&self) -> &BootstrapperConfig {
//         &self.config
//     }

//     /// Get dependencies (empty for bootstrapper as it's typically self-contained)
//     pub fn dependencies(&self) -> Vec<String> {
//         if *self.config().mode() == BootstrapperMode::SetupL1 {
//             vec!["anvil".to_string()]
//         } else {
//             vec!["anvil".to_string(), "madara".to_string(), "bootstraper_l1".to_string()]
//         }
//     }
// }
