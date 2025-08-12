use crate::services::helpers::{get_binary_path, get_file_path};
use crate::services::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::services::constants::*;

#[derive(Debug, Clone, PartialEq)]
pub enum BootstrapperMode {
    SetupL1,
    SetupL2,
}

impl std::fmt::Display for BootstrapperMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapperMode::SetupL1 => write!(f, "setup-l1"),
            BootstrapperMode::SetupL2 => write!(f, "setup-l2"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BootstrapperError {
    #[error("Bootstrapper binary not found: {0}")]
    BinaryNotFound(String),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Missing required configuration: {0}")]
    MissingConfig(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Bootstrapper execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Bootstrapper execution timed out after {0:?}")]
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

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct BootstrapperConfig {
    mode: BootstrapperMode,
    timeout: Duration,
    config_path: Option<PathBuf>,
    binary_path: PathBuf,
    logs: (bool, bool),
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl Default for BootstrapperConfig {
    fn default() -> Self {
        Self {
            mode: BootstrapperMode::SetupL1,
            timeout: BOOTSTRAPPER_SETUP_L1_TIMEOUT.clone(),
            config_path: None,
            binary_path: get_binary_path(BOOTSTRAPPER_BINARY),
            logs: (true, true),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }
}

impl BootstrapperConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for BootstrapperConfig
    pub fn builder() -> BootstrapperConfigBuilder {
        BootstrapperConfigBuilder::new()
    }

    /// Get the bootstrapper mode
    pub fn mode(&self) -> &BootstrapperMode {
        &self.mode
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Get the logs
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the configuration file path
    pub fn config_path(&self) -> Option<&PathBuf> {
        self.config_path.as_ref()
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
        let mut cmd = tokio::process::Command::new(&self.binary_path);

        // Core arguments
        cmd.arg("--mode").arg(self.mode.to_string());

        if let Some(config_path) = &self.config_path {
            cmd.arg("--config").arg(config_path);
        }

        // Additional arguments
        for arg in &self.additional_args {
            cmd.arg(arg);
        }

        // Environment variables
        for (key, value) in &self.environment_vars {
            cmd.env(key, value);
        }

        cmd
    }
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct BootstrapperConfigBuilder {
    config: BootstrapperConfig,
}

impl BootstrapperConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self { config: BootstrapperConfig::default() }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> BootstrapperConfig {
        self.config
    }

    /// Set the bootstrapper mode
    pub fn mode(mut self, mode: BootstrapperMode) -> Self {
        self.config.mode = mode;
        self
    }

    /// Set the configuration file path
    pub fn config_path(mut self, path: &str) -> Self {
        self.config.config_path = Some(get_file_path(path));
        self
    }

    /// Set the binary path
    pub fn binary_path(mut self, path: &str) -> Self {
        self.config.binary_path = get_binary_path(path);
        self
    }

    /// Add an environment variable
    pub fn env_var(mut self, key: &str, value: &str) -> Self {
        self.config.environment_vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Set the logs
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.config.logs = logs;
        self
    }

    /// Add an additional argument
    pub fn arg(mut self, arg: &str) -> Self {
        self.config.additional_args.push(arg.to_string());
        self
    }

    /// Set the timeout duration
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }
}

impl Default for BootstrapperConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
