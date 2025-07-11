use crate::servers::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_BOOTSTRAPPER_BINARY: &str = "../target/release/bootstrapper";
pub const DEFAULT_BOOTSTRAPPER_CONFIG: &str = "../bootstrapper/src/configs/devnet.json";

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
    #[error("Setup failed with exit code: {0}")]
    SetupFailed(i32),
    #[error("File system error: {0}")]
    FileSystem(#[from] std::io::Error),
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct BootstrapperConfigBuilder {
    mode: BootstrapperMode,
    timeout: Duration,
    config_path: Option<PathBuf>,
    binary_path: PathBuf,
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct BootstrapperConfig {
    mode: BootstrapperMode,
    timeout: Duration,
    config_path: Option<PathBuf>,
    binary_path: PathBuf,
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl Default for BootstrapperConfigBuilder {
    fn default() -> Self {
        Self {
            mode: BootstrapperMode::SetupL1,
            timeout: Duration::from_secs(60),
            config_path: Some(PathBuf::from(DEFAULT_BOOTSTRAPPER_CONFIG)),
            binary_path: PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }
}

impl BootstrapperConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the bootstrapper mode
    pub fn with_mode(mut self, mode: BootstrapperMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the configuration file path
    pub fn with_config_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Set the binary path
    pub fn with_binary_path<P: Into<PathBuf>>(mut self, path: Option<P>) -> Self {
        if let Some(p) = path {
            self.binary_path = p.into();
        }
        self
    }

    /// Add an environment variable
    pub fn add_env_var(mut self, key: &str, value: &str) -> Self {
        self.environment_vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Add an additional argument
    pub fn add_arg(mut self, arg: &str) -> Self {
        self.additional_args.push(arg.to_string());
        self
    }

    /// Set the timeout duration
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build the final immutable configuration
    pub fn build(self) -> BootstrapperConfig {
        BootstrapperConfig {
            mode: self.mode,
            timeout: self.timeout,
            config_path: self.config_path,
            binary_path: self.binary_path,
            environment_vars: self.environment_vars,
            additional_args: self.additional_args,
        }
    }
}

impl BootstrapperConfig {
    /// Get the bootstrapper mode
    pub fn mode(&self) -> &BootstrapperMode {
        &self.mode
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
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

    /// Convert the configuration to a command
    pub fn to_command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.binary_path);

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
