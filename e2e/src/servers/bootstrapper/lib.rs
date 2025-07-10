// =============================================================================
// BOOTSTRAPPER SERVICE - Setup utility for L1/L2 initialization
// =============================================================================
use super::{BootstrapperCMD, BootstrapperConfig, BootstrapperError, BootstrapperMode};
use crate::servers::bootstrapper::DEFAULT_BOOTSTRAPPER_BINARY;
use crate::servers::bootstrapper::DEFAULT_BOOTSTRAPPER_CONFIG;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
pub struct BootstrapperService {
    config: BootstrapperConfig,
    cmd: BootstrapperCMD,
    completed: bool,
    exit_status: Option<ExitStatus>,
}

impl BootstrapperService {
    /// Run the bootstrapper (blocking operation that completes when done)
    pub async fn run(config: BootstrapperConfig) -> Result<Self, BootstrapperError> {
        // Validate configuration
        Self::validate_config(&config)?;

        // Create the command
        let cmd = BootstrapperCMD::from_config(&config);

        // Build and execute the command
        let exit_status = Self::execute_command(&config, &cmd).await?;

        Ok(Self { config, cmd, completed: true, exit_status: Some(exit_status) })
    }

    /// Run bootstrapper with custom command
    pub async fn run_with_cmd(config: BootstrapperConfig, cmd: BootstrapperCMD) -> Result<Self, BootstrapperError> {
        Self::validate_config(&config)?;

        let exit_status = Self::execute_command(&config, &cmd).await?;

        Ok(Self { config, cmd, completed: true, exit_status: Some(exit_status) })
    }

    /// Validate the configuration
    fn validate_config(config: &BootstrapperConfig) -> Result<(), BootstrapperError> {
        // Check if config file exists
        if !config.config_path.exists() {
            return Err(BootstrapperError::MissingConfig(format!(
                "Config file does not exist: {}",
                config.config_path.display()
            )));
        }

        // Check if binary exists (if not using cargo)
        if !config.use_cargo {
            if let Some(ref binary_path) = config.binary_path {
                if !binary_path.exists() {
                    return Err(BootstrapperError::BinaryNotFound(format!(
                        "Binary not found: {}",
                        binary_path.display()
                    )));
                }
            } else {
                return Err(BootstrapperError::MissingConfig("Binary path required when not using cargo".to_string()));
            }
        }

        Ok(())
    }

    /// Execute the bootstrapper command
    async fn execute_command(
        config: &BootstrapperConfig,
        cmd: &BootstrapperCMD,
    ) -> Result<ExitStatus, BootstrapperError> {
        println!("🚀 Running bootstrapper in {} mode...", config.mode);

        let mut command = Self::build_command(config, cmd)?;

        println!("Bootstrapper Command : {:?}", command);
        // Use timeout to prevent hanging
        let result = tokio::time::timeout(config.timeout, async {
            // Spawn the process
            let mut child = command.spawn().map_err(|e| BootstrapperError::ExecutionFailed(e.to_string()))?;

            // Wait for completion
            let exit_status = child.wait().map_err(|e| BootstrapperError::ExecutionFailed(e.to_string()))?;

            Ok::<ExitStatus, BootstrapperError>(exit_status)
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!("✅ Bootstrapper {} completed successfully", config.mode);
                    Ok(exit_status)
                } else {
                    let exit_code = exit_status.code().unwrap_or(-1);
                    Err(BootstrapperError::SetupFailed(exit_code))
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                Err(BootstrapperError::ExecutionFailed(format!("Bootstrapper timed out after {:?}", config.timeout)))
            }
        }
    }

    /// Build the command to run the bootstrapper
    fn build_command(config: &BootstrapperConfig, cmd: &BootstrapperCMD) -> Result<Command, BootstrapperError> {
        let mut command = if config.use_cargo {
            let mut c = Command::new("cargo");
            c.arg("run");
            if config.release_mode {
                c.arg("--release");
            }
            c.arg("--bin").arg("bootstrapper").arg("--");
            c
        } else {
            // Use binary directly
            if let Some(ref binary_path) = config.binary_path {
                Command::new(binary_path)
            } else {
                return Err(BootstrapperError::MissingConfig("Binary path required when not using cargo".to_string()));
            }
        };

        // Add all arguments
        command.args(&cmd.args);

        // Add environment variables
        for (key, value) in &cmd.env {
            command.env(key, value);
        }

        // Set up stdio for visibility
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());

        Ok(command)
    }

    /// Run L1 setup
    pub async fn setup_l1(config_path: Option<PathBuf>) -> Result<Self, BootstrapperError> {
        let config = BootstrapperConfig {
            mode: BootstrapperMode::SetupL1,
            config_path: config_path.unwrap_or_else(|| PathBuf::from(DEFAULT_BOOTSTRAPPER_CONFIG)),
            ..Default::default()
        };

        Self::run(config).await
    }

    /// Run L2 setup
    pub async fn setup_l2(config_path: Option<PathBuf>) -> Result<Self, BootstrapperError> {
        let config = BootstrapperConfig {
            mode: BootstrapperMode::SetupL2,
            config_path: config_path.unwrap_or_else(|| PathBuf::from(DEFAULT_BOOTSTRAPPER_CONFIG)),
            ..Default::default()
        };

        Self::run(config).await
    }

    /// Run both L1 and L2 setup in sequence
    pub async fn setup_complete(config_path: Option<PathBuf>) -> Result<(Self, Self), BootstrapperError> {
        println!("🔧 Running complete bootstrapper setup (L1 + L2)...");

        // Run L1 setup first
        let l1_result = Self::setup_l1(config_path.clone()).await?;
        println!("✅ L1 setup completed");

        // Run L2 setup
        let l2_result = Self::setup_l2(config_path).await?;
        println!("✅ L2 setup completed");

        println!("✅ Complete bootstrapper setup finished");
        Ok((l1_result, l2_result))
    }

    /// Create a configuration for development environment
    pub fn devnet_config(mode: BootstrapperMode) -> BootstrapperConfig {
        BootstrapperConfig {
            mode,
            config_path: PathBuf::from("bootstrapper/src/configs/devnet.json"),
            ..Default::default()
        }
    }

    /// Create a configuration for testnet environment
    pub fn testnet_config(mode: BootstrapperMode) -> BootstrapperConfig {
        BootstrapperConfig {
            mode,
            config_path: PathBuf::from("bootstrapper/src/configs/testnet.json"),
            ..Default::default()
        }
    }

    /// Create a configuration using binary directly
    pub fn binary_config(mode: BootstrapperMode, binary_path: PathBuf, config_path: PathBuf) -> BootstrapperConfig {
        BootstrapperConfig { mode, config_path, binary_path: Some(binary_path), use_cargo: false, ..Default::default() }
    }

    /// Check if bootstrapper binary exists
    pub fn check_binary() -> Result<(), BootstrapperError> {
        let binary_path = PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY);
        if binary_path.exists() {
            Ok(())
        } else {
            Err(BootstrapperError::BinaryNotFound(format!("Default binary not found: {}", binary_path.display())))
        }
    }

    /// Check if bootstrapper can be run via cargo
    pub fn check_cargo() -> Result<(), BootstrapperError> {
        let result = std::process::Command::new("cargo").args(["check", "--bin", "bootstrapper"]).output();

        match result {
            Ok(output) => {
                if output.status.success() {
                    Ok(())
                } else {
                    Err(BootstrapperError::BinaryNotFound("Bootstrapper binary not available via cargo".to_string()))
                }
            }
            Err(e) => Err(BootstrapperError::ExecutionFailed(e.to_string())),
        }
    }

    /// Get the mode that was executed
    pub fn mode(&self) -> &BootstrapperMode {
        &self.config.mode
    }

    /// Get the config path that was used
    pub fn config_path(&self) -> &PathBuf {
        &self.config.config_path
    }

    /// Check if the bootstrapper completed successfully
    pub fn is_successful(&self) -> bool {
        self.completed && self.exit_status.map_or(false, |status| status.success())
    }

    /// Get the exit status
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exit_status
    }

    /// Check if execution completed
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    /// Get the configuration used
    pub fn config(&self) -> &BootstrapperConfig {
        &self.config
    }

    /// Get the command that was executed
    pub fn cmd(&self) -> &BootstrapperCMD {
        &self.cmd
    }

    /// Get dependencies (empty for bootstrapper as it's typically self-contained)
    pub fn dependencies(&self) -> Vec<String> {
        vec![] // Bootstrapper typically doesn't depend on other services
    }

    /// Validate that the bootstrapper setup was successful by checking expected outputs
    pub async fn validate_setup_success(&self) -> Result<bool, BootstrapperError> {
        if !self.is_successful() {
            return Ok(false);
        }

        // Here you could add specific validation logic based on what the bootstrapper should produce
        // For example, checking if certain files were created, contracts deployed, etc.

        // Basic validation - just check if it completed successfully
        Ok(true)
    }
}
