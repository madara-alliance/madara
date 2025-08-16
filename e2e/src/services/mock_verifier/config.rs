use url::Url;

use crate::services::constants::*;
use crate::services::helpers::{get_database_path, get_file_path};
use crate::services::server::ServerError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum MockVerifierDeployerError {
    #[error("Script not found: {0}")]
    ScriptNotFound(String),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Missing required configuration: {0}")]
    MissingConfig(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Deployment script execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Deployment failed with exit code: {0}")]
    DeploymentFailed(i32),
    #[error("File system error: {0}")]
    FileSystem(#[from] std::io::Error),
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct MockVerifierDeployerConfig {
    timeout: Duration,
    script_path: PathBuf,
    private_key: String,
    l1_url: Url,
    mock_gps_verifier_path: String,
    verifier_file_path: PathBuf,
    logs: (bool, bool),
    environment_vars: HashMap<String, String>,
    additional_args: Vec<String>,
}

impl Default for MockVerifierDeployerConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(MOCK_VERIFIER_TIMEOUT_SECS),
            script_path: get_file_path(MOCK_VERIFIER_DEPLOYER_SCRIPT),
            private_key: ANVIL_PRIVATE_KEY.to_string(),
            l1_url: Url::parse(format!("http://{}:{}", DEFAULT_SERVICE_HOST, ANVIL_PORT).as_str()).unwrap(),
            mock_gps_verifier_path: get_file_path(MOCK_VERIFIER_CONTRACT).to_string_lossy().to_string(),
            verifier_file_path: get_database_path(DATA_DIR, MOCK_VERIFIER_ADDRESS_FILE),
            logs: (true, true),
            environment_vars: HashMap::new(),
            additional_args: Vec::new(),
        }
    }
}

impl MockVerifierDeployerConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for MockVerifierDeployerConfig
    pub fn builder() -> MockVerifierDeployerConfigBuilder {
        MockVerifierDeployerConfigBuilder::new()
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Get the script path
    pub fn script_path(&self) -> &PathBuf {
        &self.script_path
    }

    /// Get the logs
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the private key
    pub fn private_key(&self) -> &str {
        &self.private_key
    }

    /// Get the l1 URL
    pub fn l1_url(&self) -> &Url {
        &self.l1_url
    }

    /// Get the mock GPS verifier path
    pub fn mock_gps_verifier_path(&self) -> &str {
        &self.mock_gps_verifier_path
    }

    /// Get the verifier file path
    pub fn verifier_file_path(&self) -> &PathBuf {
        &self.verifier_file_path
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
        let mut command_string = self.script_path.to_string_lossy().to_string();

        // Create command that first makes the script executable, then runs it
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c");

        // Add script arguments
        command_string.push_str(&format!(" --private-key '{}'", self.private_key));
        command_string.push_str(&format!(" --anvil-url '{}'", self.l1_url));
        command_string.push_str(&format!(" --mock-gps-verifier-path '{}'", self.mock_gps_verifier_path));
        command_string.push_str(&format!(" --verifier-file-name '{}'", self.verifier_file_path.to_string_lossy()));

        // Additional arguments
        for arg in &self.additional_args {
            command_string.push_str(&format!(" '{}'", arg));
        }

        cmd.arg(command_string);

        // Environment variables
        for (key, value) in &self.environment_vars {
            cmd.env(key, value);
        }

        cmd
    }
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct MockVerifierDeployerConfigBuilder {
    config: MockVerifierDeployerConfig,
}

impl MockVerifierDeployerConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self { config: MockVerifierDeployerConfig::default() }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> MockVerifierDeployerConfig {
        self.config
    }

    /// Set the script path
    pub fn script_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.config.script_path = path.into();
        self
    }

    /// Set the private key
    pub fn private_key<S: Into<String>>(mut self, key: S) -> Self {
        self.config.private_key = key.into();
        self
    }

    /// Set the anvil URL
    pub fn l1_url(mut self, url: Url) -> Self {
        self.config.l1_url = url;
        self
    }

    /// Set the logs
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.config.logs = logs;
        self
    }

    /// Set the mock GPS verifier path
    pub fn mock_gps_verifier_path<S: Into<String>>(mut self, path: S) -> Self {
        self.config.mock_gps_verifier_path = path.into();
        self
    }

    /// Set the verifier file path
    pub fn verifier_file_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.config.verifier_file_path = path.into();
        self
    }

    /// Add an environment variable
    pub fn env_var(mut self, key: &str, value: &str) -> Self {
        self.config.environment_vars.insert(key.to_string(), value.to_string());
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

impl Default for MockVerifierDeployerConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
