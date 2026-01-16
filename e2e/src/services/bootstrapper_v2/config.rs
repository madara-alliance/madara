use crate::services::helpers::{get_binary_path, get_file_path};
use crate::services::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::services::constants::*;

// =============================================================================
// MODE ENUM
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum BootstrapperV2Mode {
    SetupBase,
    SetupMadara,
}

impl std::fmt::Display for BootstrapperV2Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapperV2Mode::SetupBase => write!(f, "setup-base"),
            BootstrapperV2Mode::SetupMadara => write!(f, "setup-madara"),
        }
    }
}

// =============================================================================
// ERROR TYPES
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum BootstrapperV2Error {
    #[error("Bootstrapper V2 binary not found: {0}")]
    BinaryNotFound(String),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Missing required configuration: {0}")]
    MissingConfig(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Bootstrapper V2 execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Bootstrapper V2 execution timed out after {0:?}")]
    ExecutionTimedOut(Duration),
    #[error("Setup failed with exit code: {0}")]
    SetupFailedWithCode(i32),
    #[error("Setup failed - process terminated by signal: {0}")]
    SetupFailedWithSignal(String),
    #[error("Config read error: {0}")]
    ConfigReadWriteError(#[from] std::io::Error),
    #[error("Config parse error: {0}")]
    ConfigParseError(#[from] serde_json::Error),
}

// =============================================================================
// MODE-SPECIFIC ARGUMENTS
// =============================================================================

/// Arguments specific to the setup-base subcommand
#[derive(Debug, Clone)]
pub struct SetupBaseArgs {
    /// Path to the configuration file (--config-path)
    pub config_path: PathBuf,
    /// Path to output the deployed addresses (--addresses-output-path)
    pub addresses_output_path: PathBuf,
    /// Private key for deployment (BASE_LAYER_PRIVATE_KEY env var)
    pub private_key: String,
}

/// Arguments specific to the setup-madara subcommand
#[derive(Debug, Clone)]
pub struct SetupMadaraArgs {
    /// Path to the configuration file (--config-path)
    pub config_path: PathBuf,
    /// Path to the base layer addresses file (--base-addresses-path)
    pub base_addresses_path: PathBuf,
    /// Path to output the Madara deployment addresses (--output-path)
    pub output_path: PathBuf,
    /// Private key for Madara deployment (MADARA_PRIVATE_KEY env var)
    pub private_key: String,
    /// Base layer private key (BASE_LAYER_PRIVATE_KEY env var)
    pub base_layer_private_key: String,
}

/// Combined mode arguments
#[derive(Debug, Clone)]
pub enum ModeArgs {
    SetupBase(SetupBaseArgs),
    SetupMadara(SetupMadaraArgs),
}

// =============================================================================
// CONFIGURATION
// =============================================================================

/// Final immutable configuration for BootstrapperV2
#[derive(Debug, Clone)]
pub struct BootstrapperV2Config {
    mode_args: ModeArgs,
    timeout: Duration,
    binary_path: PathBuf,
    logs: (bool, bool),
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl BootstrapperV2Config {
    /// Create a builder for SetupBase configuration
    pub fn setup_base() -> SetupBaseBuilder {
        SetupBaseBuilder::new()
    }

    /// Create a builder for SetupMadara configuration
    pub fn setup_madara() -> SetupMadaraBuilder {
        SetupMadaraBuilder::new()
    }

    /// Get the bootstrapper mode
    pub fn mode(&self) -> BootstrapperV2Mode {
        match &self.mode_args {
            ModeArgs::SetupBase(_) => BootstrapperV2Mode::SetupBase,
            ModeArgs::SetupMadara(_) => BootstrapperV2Mode::SetupMadara,
        }
    }

    /// Get the mode arguments
    pub fn mode_args(&self) -> &ModeArgs {
        &self.mode_args
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Get the logs configuration (stdout, stderr)
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the binary path
    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }

    /// Get the environment variables
    pub fn environment_vars(&self) -> &HashMap<String, String> {
        &self.environment_vars
    }

    /// Get the additional arguments
    pub fn additional_args(&self) -> &[String] {
        &self.additional_args
    }

    /// Convert the configuration to a tokio command
    pub fn to_command(&self) -> tokio::process::Command {
        use super::super::constants::REPO_ROOT;

        // Use REPO_ROOT which is the correct path to the repository root
        // (tests may run from different directories like e2e/)
        let repo_root = REPO_ROOT.clone();

        // Set working directory to bootstrapper-v2/ since the binary uses relative paths
        // for contract artifacts that are relative to that directory
        let working_dir = repo_root.join("bootstrapper-v2");

        // Binary path is already absolute (set by get_binary_path in builders)
        let mut cmd = tokio::process::Command::new(&self.binary_path);
        cmd.current_dir(working_dir);

        match &self.mode_args {
            ModeArgs::SetupBase(args) => {
                // Paths are already absolute (set by builders using get_file_path)
                cmd.arg("setup-base")
                    .arg("--config-path")
                    .arg(&args.config_path)
                    .arg("--addresses-output-path")
                    .arg(&args.addresses_output_path)
                    .env("BASE_LAYER_PRIVATE_KEY", &args.private_key);
            }
            ModeArgs::SetupMadara(args) => {
                // Paths are already absolute (set by builders using get_file_path)
                cmd.arg("setup-madara")
                    .arg("--config-path")
                    .arg(&args.config_path)
                    .arg("--base-addresses-path")
                    .arg(&args.base_addresses_path)
                    .arg("--output-path")
                    .arg(&args.output_path)
                    .env("MADARA_PRIVATE_KEY", &args.private_key)
                    .env("BASE_LAYER_PRIVATE_KEY", &args.base_layer_private_key);
            }
        }

        // Additional arguments
        for arg in &self.additional_args {
            cmd.arg(arg);
        }

        // Additional environment variables
        for (key, value) in &self.environment_vars {
            cmd.env(key, value);
        }

        cmd
    }
}

// =============================================================================
// SETUP BASE BUILDER
// =============================================================================

#[derive(Debug, Clone)]
pub struct SetupBaseBuilder {
    config_path: Option<PathBuf>,
    addresses_output_path: Option<PathBuf>,
    private_key: Option<String>,
    timeout: Duration,
    binary_path: PathBuf,
    logs: (bool, bool),
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl SetupBaseBuilder {
    pub fn new() -> Self {
        Self {
            config_path: None,
            addresses_output_path: None,
            private_key: None,
            timeout: *BOOTSTRAPPER_V2_SETUP_BASE_TIMEOUT,
            binary_path: get_binary_path(BOOTSTRAPPER_V2_BINARY),
            logs: (true, true),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }

    /// Set the configuration file path
    pub fn config_path(mut self, path: &str) -> Self {
        self.config_path = Some(get_file_path(path));
        self
    }

    /// Set the addresses output path
    pub fn addresses_output_path(mut self, path: &str) -> Self {
        self.addresses_output_path = Some(get_file_path(path));
        self
    }

    /// Set the private key for deployment
    pub fn private_key(mut self, key: &str) -> Self {
        self.private_key = Some(key.to_string());
        self
    }

    /// Set the timeout duration
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the binary path
    pub fn binary_path(mut self, path: &str) -> Self {
        self.binary_path = get_binary_path(path);
        self
    }

    /// Set the logs configuration
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.logs = logs;
        self
    }

    /// Add an environment variable
    pub fn env_var(mut self, key: &str, value: &str) -> Self {
        self.environment_vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Add an additional argument
    pub fn arg(mut self, arg: &str) -> Self {
        self.additional_args.push(arg.to_string());
        self
    }

    /// Build the final configuration
    pub fn build(self) -> Result<BootstrapperV2Config, BootstrapperV2Error> {
        let config_path = self.config_path.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("config_path is required for setup-base".to_string())
        })?;

        let addresses_output_path = self.addresses_output_path.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("addresses_output_path is required for setup-base".to_string())
        })?;

        let private_key = self.private_key.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("private_key is required for setup-base".to_string())
        })?;

        Ok(BootstrapperV2Config {
            mode_args: ModeArgs::SetupBase(SetupBaseArgs {
                config_path,
                addresses_output_path,
                private_key,
            }),
            timeout: self.timeout,
            binary_path: self.binary_path,
            logs: self.logs,
            environment_vars: self.environment_vars,
            additional_args: self.additional_args,
        })
    }
}

impl Default for SetupBaseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// SETUP MADARA BUILDER
// =============================================================================

#[derive(Debug, Clone)]
pub struct SetupMadaraBuilder {
    config_path: Option<PathBuf>,
    base_addresses_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    private_key: Option<String>,
    base_layer_private_key: Option<String>,
    timeout: Duration,
    binary_path: PathBuf,
    logs: (bool, bool),
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl SetupMadaraBuilder {
    pub fn new() -> Self {
        Self {
            config_path: None,
            base_addresses_path: None,
            output_path: None,
            private_key: None,
            base_layer_private_key: None,
            timeout: *BOOTSTRAPPER_V2_SETUP_MADARA_TIMEOUT,
            binary_path: get_binary_path(BOOTSTRAPPER_V2_BINARY),
            logs: (true, true),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }

    /// Set the configuration file path
    pub fn config_path(mut self, path: &str) -> Self {
        self.config_path = Some(get_file_path(path));
        self
    }

    /// Set the base addresses path
    pub fn base_addresses_path(mut self, path: &str) -> Self {
        self.base_addresses_path = Some(get_file_path(path));
        self
    }

    /// Set the output path
    pub fn output_path(mut self, path: &str) -> Self {
        self.output_path = Some(get_file_path(path));
        self
    }

    /// Set the private key for Madara deployment
    pub fn private_key(mut self, key: &str) -> Self {
        self.private_key = Some(key.to_string());
        self
    }

    /// Set the base layer private key
    pub fn base_layer_private_key(mut self, key: &str) -> Self {
        self.base_layer_private_key = Some(key.to_string());
        self
    }

    /// Set the timeout duration
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the binary path
    pub fn binary_path(mut self, path: &str) -> Self {
        self.binary_path = get_binary_path(path);
        self
    }

    /// Set the logs configuration
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.logs = logs;
        self
    }

    /// Add an environment variable
    pub fn env_var(mut self, key: &str, value: &str) -> Self {
        self.environment_vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Add an additional argument
    pub fn arg(mut self, arg: &str) -> Self {
        self.additional_args.push(arg.to_string());
        self
    }

    /// Build the final configuration
    pub fn build(self) -> Result<BootstrapperV2Config, BootstrapperV2Error> {
        let config_path = self.config_path.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("config_path is required for setup-madara".to_string())
        })?;

        let base_addresses_path = self.base_addresses_path.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("base_addresses_path is required for setup-madara".to_string())
        })?;

        let output_path = self.output_path.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("output_path is required for setup-madara".to_string())
        })?;

        let private_key = self.private_key.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("private_key is required for setup-madara".to_string())
        })?;

        let base_layer_private_key = self.base_layer_private_key.ok_or_else(|| {
            BootstrapperV2Error::MissingConfig("base_layer_private_key is required for setup-madara".to_string())
        })?;

        Ok(BootstrapperV2Config {
            mode_args: ModeArgs::SetupMadara(SetupMadaraArgs {
                config_path,
                base_addresses_path,
                output_path,
                private_key,
                base_layer_private_key,
            }),
            timeout: self.timeout,
            binary_path: self.binary_path,
            logs: self.logs,
            environment_vars: self.environment_vars,
            additional_args: self.additional_args,
        })
    }
}

impl Default for SetupMadaraBuilder {
    fn default() -> Self {
        Self::new()
    }
}
