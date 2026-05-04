use crate::services::helpers::{get_binary_path, get_file_path};
use crate::services::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::services::constants::*;

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyBootstrapperMode {
    SetupL1,
    SetupL2,
}

impl std::fmt::Display for LegacyBootstrapperMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LegacyBootstrapperMode::SetupL1 => write!(f, "setup-l1"),
            LegacyBootstrapperMode::SetupL2 => write!(f, "setup-l2"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LegacyBootstrapperError {
    #[error("Legacy bootstrapper binary not found: {0}")]
    BinaryNotFound(String),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Missing required configuration: {0}")]
    MissingConfig(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Legacy bootstrapper execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Legacy bootstrapper execution timed out after {0:?}")]
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
pub struct LegacyBootstrapperConfig {
    mode: LegacyBootstrapperMode,
    timeout: Duration,
    config_path: Option<PathBuf>,
    binary_path: PathBuf,
    logs: (bool, bool),
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl Default for LegacyBootstrapperConfig {
    fn default() -> Self {
        Self {
            mode: LegacyBootstrapperMode::SetupL1,
            timeout: *LEGACY_BOOTSTRAPPER_SETUP_L1_TIMEOUT,
            config_path: None,
            binary_path: get_binary_path(LEGACY_BOOTSTRAPPER_BINARY),
            logs: (true, true),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }
}

impl LegacyBootstrapperConfig {
    /// Create a new legacy bootstrapper configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for LegacyBootstrapperConfig
    pub fn builder() -> LegacyBootstrapperConfigBuilder {
        LegacyBootstrapperConfigBuilder::new()
    }

    /// Get the legacy bootstrapper mode
    pub fn mode(&self) -> &LegacyBootstrapperMode {
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
pub struct LegacyBootstrapperConfigBuilder {
    config: LegacyBootstrapperConfig,
}

impl LegacyBootstrapperConfigBuilder {
    /// Create a new legacy bootstrapper configuration builder with default values
    pub fn new() -> Self {
        Self { config: LegacyBootstrapperConfig::default() }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> LegacyBootstrapperConfig {
        self.config
    }

    /// Set the legacy bootstrapper mode
    pub fn mode(mut self, mode: LegacyBootstrapperMode) -> Self {
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

impl Default for LegacyBootstrapperConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
