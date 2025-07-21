// =============================================================================
// MOCK PROVER SERVICE - Using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};

pub struct MockProverService {
    server: Server,
    config: MockProverConfig,
}

impl MockProverService {
    /// Start the mock prover service
    pub async fn start(config: MockProverConfig) -> Result<Self, MockProverError> {
        let command = config.to_command();
        let port = config.port();

        println!("Running mock prover with command: {:?}", command);

        // Create server config
        let server_config = ServerConfig {
            rpc_port: Some(port),
            service_name: "MockProver".to_string(),
            logs: config.logs(),
            connection_attempts: 30,
            connection_delay_ms: 1000,
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(MockProverError::Server)?;

        Ok(Self { server, config })
    }

    /// Get the dependencies required by the mock prover
    pub fn dependencies(&self) -> Vec<String> {
        vec![
            // Mock prover typically has minimal dependencies
            // Add any external dependencies here if needed
        ]
    }

    /// Get the port number
    pub fn port(&self) -> u16 {
        self.config.port()
    }

    /// Get the configuration used
    pub fn config(&self) -> &MockProverConfig {
        &self.config
    }

    /// Get the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }
}
