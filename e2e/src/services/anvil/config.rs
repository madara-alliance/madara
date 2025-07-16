use crate::services::server::ServerError;
use tokio::process::Command;

pub const ANVIL_DEFAULT_DATABASE_NAME: &str = "anvil.json";

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
    port: u16,
    fork_url: Option<String>,
    load_state: Option<String>,
    dump_state: Option<String>,
    block_time: Option<f64>,
}

impl Default for AnvilConfig {
    fn default() -> Self {
        Self {
            port: 8545,
            fork_url: None,
            load_state: None,
            dump_state: None,
            block_time: None,
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

    /// Get the fork URL
    pub fn fork_url(&self) -> Option<&str> {
        self.fork_url.as_deref()
    }

    /// Get the load state path
    pub fn load_state(&self) -> Option<&str> {
        self.load_state.as_deref()
    }

    /// Get the dump state path
    pub fn dump_state(&self) -> Option<&str> {
        self.dump_state.as_deref()
    }

    /// Get the block time in seconds
    pub fn block_time(&self) -> Option<f64> {
        self.block_time
    }

    /// Build the final immutable configuration
    pub fn to_command(&self) -> Command {
        let mut command = Command::new("anvil");
        command.arg("--port").arg(self.port().to_string());

        if let Some(fork_url) = self.fork_url() {
            command.arg("--fork-url").arg(fork_url);
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
        Self {
            config: AnvilConfig::default(),
        }
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
    pub fn fork_url<S: Into<String>>(mut self, url: S) -> Self {
        self.config.fork_url = Some(url.into());
        self
    }

    /// Set the database file to load state from
    pub fn load_state<S: Into<String>>(mut self, path: S) -> Self {
        self.config.load_state = Some(path.into());
        self
    }

    /// Set the database file to dump state to
    pub fn dump_state<S: Into<String>>(mut self, path: S) -> Self {
        self.config.dump_state = Some(path.into());
        self
    }

    /// Set the block time in seconds (must be non-negative, can be decimal)
    pub fn block_time(mut self, seconds: f64) -> Self {
        if seconds >= 0.0 {
            self.config.block_time = Some(seconds);
        }
        self
    }
}

impl Default for AnvilConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
