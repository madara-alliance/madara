use crate::services::constants::DEFAULT_SERVICE_HOST;
use crate::services::{constants::*, server::ServerError};
use std::path::PathBuf;
use tokio::process::Command;
use url::Url;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AnvilError {
    #[error("Anvil is not installed on the system")]
    NotInstalled,
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
}

// Final immutable configuration
#[derive(Debug, Clone)]
pub struct AnvilConfig {
    fork_url: Option<Url>,
    load_state: Option<PathBuf>,
    dump_state: Option<PathBuf>,
    block_time: Option<f64>,
    // Server Config
    port: u16,
    logs: (bool, bool),
}

impl Default for AnvilConfig {
    fn default() -> Self {
        Self {
            fork_url: None,
            load_state: None,
            dump_state: None,
            block_time: Some(ANVIL_BLOCK_TIME),
            // Server Config
            port: ANVIL_PORT,
            logs: (true, true),
        }
    }
}

impl AnvilConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for AnvilConfig
    pub fn builder() -> AnvilConfigBuilder {
        AnvilConfigBuilder::new()
    }

    /// Get the port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the logs configuration
    pub fn logs(&self) -> (bool, bool) {
        self.logs
    }

    /// Get the fork URL
    pub fn fork_url(&self) -> Option<&Url> {
        self.fork_url.as_ref()
    }

    /// Get the load state path
    pub fn load_state(&self) -> Option<&PathBuf> {
        self.load_state.as_ref()
    }

    /// Get the dump state path
    pub fn dump_state(&self) -> Option<&PathBuf> {
        self.dump_state.as_ref()
    }

    /// Get the block time in seconds
    pub fn block_time(&self) -> Option<f64> {
        self.block_time
    }

    /// Get the endpoint
    pub fn endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", DEFAULT_SERVICE_HOST, self.port())).unwrap()
    }

    /// Build the final immutable configuration
    pub fn to_command(&self) -> Command {
        let mut command = Command::new("anvil");
        command.arg("--port").arg(self.port().to_string());

        if let Some(fork_url) = self.fork_url() {
            command.arg("--fork-url").arg(fork_url.to_string());
        }

        if let Some(load_state) = self.load_state() {
            command.arg("--load-state").arg(load_state);
        }

        if let Some(dump_state) = self.dump_state() {
            command.arg("--dump-state").arg(dump_state);
        }

        if let Some(block_time) = self.block_time() {
            command.arg("--block-time").arg(block_time.to_string());
        }

        command
    }
}

// Builder type that allows configuration
#[derive(Debug, Clone)]
pub struct AnvilConfigBuilder {
    config: AnvilConfig,
}

impl AnvilConfigBuilder {
    /// Create a new configuration builder with default values
    pub fn new() -> Self {
        Self { config: AnvilConfig::default() }
    }

    /// Build the final immutable configuration
    pub fn build(self) -> AnvilConfig {
        self.config
    }

    /// Set the port (default: 8545)
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the fork URL for forking from an existing network
    pub fn fork_url(mut self, url: &Url) -> Self {
        self.config.fork_url = Some(url.to_owned());
        self
    }

    /// Set the database file to load state from
    pub fn load_state<S: AsRef<std::path::Path>>(mut self, path: S) -> Self {
        self.config.load_state = Some(REPO_ROOT.clone().join(path));
        self
    }

    /// Set the database file to dump state to
    pub fn dump_state<S: AsRef<std::path::Path>>(mut self, path: S) -> Self {
        self.config.dump_state = Some(REPO_ROOT.clone().join(path));
        self
    }

    /// Set the block time in seconds (must be non-negative, can be decimal)
    pub fn block_time(mut self, seconds: f64) -> Self {
        if seconds >= 0.0 {
            self.config.block_time = Some(seconds);
        }
        self
    }

    /// Set the logs
    pub fn logs(mut self, logs: (bool, bool)) -> Self {
        self.config.logs = logs;
        self
    }
}

impl Default for AnvilConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
