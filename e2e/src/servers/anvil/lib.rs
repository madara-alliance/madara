// =============================================================================
// ANVIL SERVICE - Spawns a new Anvil service with the given configuration
// =============================================================================

use super::util::{AnvilConfig, AnvilError};
use crate::servers::server::{Server, ServerConfig};
use std::process::Command;

// Anvil service that uses the generic Server
pub struct AnvilService {
    server: Server,
    config: AnvilConfig,
}

impl AnvilService {
    /// Start a new Anvil service with the given configuration
    pub async fn start(config: AnvilConfig) -> Result<Self, AnvilError> {
        // Validate that anvil is present in the system
        if !Self::check_anvil_installed() {
            return Err(AnvilError::NotInstalled);
        }

        // Build the anvil command
        let command = Self::build_command(&config);

        // Create server config
        // The port and host should be taken from the service ideally !
        // what if the port is not available ?
        let server_config = ServerConfig { port: config.port, host: config.host.clone(), ..Default::default() };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(|err| AnvilError::Server(err))?;

        Ok(Self { server, config })
    }

    /// Build the anvil command with all arguments
    fn build_command(config: &AnvilConfig) -> Command {
        let mut command = Command::new("anvil");
        command.arg("--port").arg(config.port.to_string());
        command.arg("--host").arg(&config.host);

        if let Some(fork_url) = &config.fork_url {
            command.arg("--fork-url").arg(fork_url);
        }

        if let Some(load_db) = &config.load_state {
            command.arg("--load-state").arg(load_db);
        }

        if let Some(dump_db) = &config.dump_state {
            command.arg("--dump-state").arg(dump_db);
        }

        command
    }

    /// Check if Anvil is installed on the system
    fn check_anvil_installed() -> bool {
        Command::new("anvil").arg("--version").output().is_ok()
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn dependencies(&self) -> Option<Vec<String>> {
        Some(vec![])
    }
}
