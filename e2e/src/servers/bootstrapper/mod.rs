// =============================================================================
// BOOTSTRAPPER SERVICE - Setup utility for L1/L2 initialization
// =============================================================================

pub mod config;
// Re-export common utilities
pub use config::*;

use std::path::PathBuf;
use std::process::ExitStatus;

pub struct BootstrapperService {
    config: BootstrapperConfig,
}

impl BootstrapperService {
    /// Create a new bootstrapper service with the given configuration
    pub fn new(config: BootstrapperConfig) -> Result<Self, BootstrapperError> {
        Ok(Self { config })
    }

    /// Execute the bootstrapper command
    pub async fn run(&self) -> Result<ExitStatus, BootstrapperError> {
        println!("🚀 Running bootstrapper in {} mode...", self.config.mode());

        let mut command = self.config.to_command();
        println!("Bootstrapper Command : {:?}", command);

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(self.config.timeout(), async {
            // Spawn the process
            let mut child = command
                .spawn()
                .map_err(|e| BootstrapperError::ExecutionFailed(e.to_string()))?;

            // Wait for completion
            let exit_status = child
                .wait()
                .map_err(|e| BootstrapperError::ExecutionFailed(e.to_string()))?;

            Ok::<ExitStatus, BootstrapperError>(exit_status)
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!("✅ Bootstrapper {} completed successfully", self.config.mode());
                    Ok(exit_status)
                } else {
                    let exit_code = exit_status.code().unwrap_or(-1);
                    Err(BootstrapperError::SetupFailed(exit_code))
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(BootstrapperError::ExecutionFailed(format!(
                "Bootstrapper timed out after {:?}",
                self.config.timeout()
            ))),
        }
    }

    /// Check if bootstrapper binary exists
    pub fn check_binary() -> Result<(), BootstrapperError> {
        let binary_path = PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY);
        if binary_path.exists() {
            Ok(())
        } else {
            Err(BootstrapperError::BinaryNotFound(format!(
                "Default binary not found: {}",
                binary_path.display()
            )))
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

    /// Get dependencies (empty for bootstrapper as it's typically self-contained)
    pub fn dependencies(&self) -> Vec<String> {
        if *self.config().mode() == BootstrapperMode::SetupL1 {
            vec!["anvil".to_string()]
        } else {
            vec!["anvil".to_string(), "madara".to_string(), "bootstraper_l1".to_string()]
        }
    }
}
