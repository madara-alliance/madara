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
            service_name: "Mock Prover".to_string(),
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(MockProverError::Server)?;

        Ok(Self { server, config })
    }

    /// Get the port number
    pub fn port(&self) -> u16 {
        self.config.port()
    }

    pub fn stop(&mut self) -> Result<(), MockProverError> {
        println!("☠️ Stopping Mock Prover");
        self.server.stop().map_err(|err| MockProverError::Server(err))
    }

    /// Get the configuration used
    pub fn config(&self) -> &MockProverConfig {
        &self.config
    }

    /// Get the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn stop(&mut self) -> Result<(), MockProverError> {
        println!("☠️ Stopping Mock Prover");
        self.server.stop().map_err(MockProverError::Server)?;
        Ok(())
    }
}
