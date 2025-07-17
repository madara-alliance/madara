use std::path::PathBuf;
use tokio::process::Command;

pub const DEFAULT_MOCK_PROVER_BINARY: &str = "../target/release/mock-atlantic-server";

#[derive(Debug, thiserror::Error)]
pub enum MockProverError {
    #[error("Server error: {0}")]
    Server(#[from] crate::services::server::ServerError),
    #[error("Mock prover execution failed: {0}")]
    ExecutionFailed(String),
}

#[derive(Debug, Clone)]
pub struct MockProverConfig {
    binary_path: PathBuf,
    port: u16,
}

impl MockProverConfig {
    /// Create a new configuration with the specified port
    pub fn new(port: u16) -> Self {
        Self {
            binary_path: PathBuf::from(DEFAULT_MOCK_PROVER_BINARY),
            port,
        }
    }

    /// Create a builder for MockProverConfig
    pub fn builder() -> MockProverConfigBuilder {
        MockProverConfigBuilder::new()
    }

    /// Get the port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the binary path
    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }

    /// Convert configuration to tokio Command
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.binary_path);
        command.arg(self.port.to_string());
        command
    }
}

/// Builder for MockProverConfig
#[derive(Debug, Clone)]
pub struct MockProverConfigBuilder {
    binary_path: PathBuf,
    port: Option<u16>,
}

impl MockProverConfigBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            binary_path: PathBuf::from(DEFAULT_MOCK_PROVER_BINARY),
            port: None,
        }
    }

    /// Set the binary path
    pub fn binary_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.binary_path = path.into();
        self
    }

    /// Set the port
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Build the final configuration
    pub fn build(self) -> Result<MockProverConfig, MockProverError> {
        let port = self.port.ok_or_else(|| {
            MockProverError::ExecutionFailed("Port must be specified".to_string())
        })?;

        Ok(MockProverConfig {
            binary_path: self.binary_path,
            port,
        })
    }
}

impl Default for MockProverConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
