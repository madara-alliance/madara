// =============================================================================
// ANVIL SERVICE - Spawns a new Anvil service with the given configuration
// =============================================================================

pub mod config;
// Re-export common utilities
pub use config::*;

use crate::servers::server::{Server, ServerConfig};

use super::server::ServiceAddress;

// Anvil service that uses the generic Server
pub struct AnvilService {
    server: Server,
}

impl AnvilService {
    /// Start a new Anvil service with the given configuration
    pub async fn start(config: AnvilConfig) -> Result<Self, AnvilError> {

        // Build the anvil command
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            service_address : Some(ServiceAddress {
                port: config.port(),
                host: config.host().to_string(),
            }),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(AnvilError::Server)?;

        Ok(Self { server })
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn stop(mut self) -> Result<(), AnvilError> {
        println!("☠️ Stopping Anvil");
        self.server.stop().map_err(|err| AnvilError::Server(err))
    }

}
