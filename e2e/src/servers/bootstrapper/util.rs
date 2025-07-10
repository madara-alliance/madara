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

#[derive(Debug, Clone)]
pub struct BootstrapperConfig {
    pub mode: BootstrapperMode,
    pub config_path: PathBuf,
    pub binary_path: Option<PathBuf>,
    pub use_cargo: bool,
    pub release_mode: bool,
    pub environment_vars: HashMap<String, String>,
    pub additional_args: Vec<String>,
    pub timeout: Duration,
}

impl Default for BootstrapperConfig {
    fn default() -> Self {
        Self {
            mode: BootstrapperMode::SetupL1,
            config_path: PathBuf::from(DEFAULT_BOOTSTRAPPER_CONFIG),
            binary_path: Some(PathBuf::from(DEFAULT_BOOTSTRAPPER_BINARY)),
            use_cargo: false,
            release_mode: true,
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
            timeout: Duration::from_secs(300), // 5 minutes
        }
    }
}

pub struct BootstrapperCMDBuilder {
    args: Vec<String>,
    env: HashMap<String, String>,
}

impl BootstrapperCMDBuilder {
    pub fn new() -> Self {
        Self { args: Vec::new(), env: HashMap::new() }
    }

    pub fn with_config(config: &BootstrapperConfig) -> Self {
        let mut builder = Self::new();
        builder.build_from_config(config);
        builder
    }

    fn build_from_config(&mut self, config: &BootstrapperConfig) {
        // Core arguments
        self.add_arg("--mode", &config.mode.to_string());
        self.add_arg("--config", config.config_path.to_string_lossy().as_ref());

        // Additional arguments
        for arg in &config.additional_args {
            self.args.push(arg.clone());
        }

        // Environment variables
        for (key, value) in &config.environment_vars {
            self.env.insert(key.clone(), value.clone());
        }
    }

    pub fn add_arg(&mut self, key: &str, value: &str) -> &mut Self {
        self.args.push(key.to_string());
        self.args.push(value.to_string());
        self
    }

    pub fn add_flag(&mut self, flag: &str) -> &mut Self {
        self.args.push(flag.to_string());
        self
    }

    pub fn add_env(&mut self, key: &str, value: &str) -> &mut Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn build(&self) -> BootstrapperCMD {
        BootstrapperCMD { args: self.args.clone(), env: self.env.clone() }
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapperCMD {
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl BootstrapperCMD {
    pub fn new(args: Vec<String>, env: HashMap<String, String>) -> Self {
        Self { args, env }
    }

    pub fn from_config(config: &BootstrapperConfig) -> Self {
        BootstrapperCMDBuilder::with_config(config).build()
    }
}
