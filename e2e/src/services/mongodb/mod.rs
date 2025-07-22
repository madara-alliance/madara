// =============================================================================
// MONGODB SERVICE - Using Docker and generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::docker::{DockerError, DockerServer};
use crate::services::server::{Server, ServerConfig};
use reqwest::Url;

use crate::services::helpers::DEFAULT_DATA_DIR;
use crate::services::server::DEFAULT_MONGODB_DIR;
use crate::services::server::DEFAULT_SERVICE_HOST;
use tokio::process::Command;

pub struct MongoService {
    server: Server,
    config: MongoConfig,
}

impl MongoService {
    /// Start a new MongoDB service
    /// Will panic if MongoDB is already running as per pattern
    pub async fn start(config: MongoConfig) -> Result<Self, MongoError> {
        // Validate Docker is running
        if !DockerServer::is_docker_running().await {
            return Err(MongoError::Docker(DockerError::NotRunning));
        }

        // Check if container is already running - PANIC as per pattern
        if DockerServer::is_container_running(config.container_name()).await? {
            panic!(
                "MongoDB container '{}' is already running on port {}. Please stop it first.",
                config.container_name(),
                config.port()
            );
        }

        // Check if port is in use
        if DockerServer::is_port_in_use(config.port()) {
            return Err(MongoError::PortInUse(config.port()));
        }

        // Clean up any existing stopped container with the same name
        if DockerServer::does_container_exist(config.container_name()).await? {
            DockerServer::remove_container(config.container_name()).await?;
        }

        // Build the docker command
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            rpc_port: Some(config.port()),
            service_name: "MongoDB".to_string(),
            connection_attempts: 30, // MongoDB usually starts quickly
            connection_delay_ms: 1000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(|e| MongoError::Docker(DockerError::Server(e)))?;

        Ok(Self { server, config })
    }

    /// Get the endpoint URL for the MongoDB service
    pub fn endpoint(&self) -> Url {
        // MongoDB doesn't use HTTP, but we'll return the TCP endpoint
        Url::parse(&format!("mongodb://{}:{}", DEFAULT_SERVICE_HOST, self.config().port())).unwrap()
    }

    /// Get the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Get the configuration used
    pub fn config(&self) -> &MongoConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), MongoError> {
        println!("☠️ Stopping MongoDB");
        self.server.stop().map_err(|err| MongoError::Server(err))
    }

    /// Get dependencies (Docker is required)
    pub fn dependencies(&self) -> Vec<String> {
        vec!["docker".to_string()]
    }
}

// MongoDump and MongoRestore impl from within the docker container
impl MongoService {
    /// MongoDump
    /// docker exec <container_name> mongodump --host "localhost:27017" --db orchestrator_3 --out /backup
    pub async fn dump_db(&self, database_name: &str) -> Result<(), MongoError> {
        let command = format!(
            "docker exec {} mongodump --host \"{}:{}\" --db {} --out {}",
            self.config().container_name(),
            DEFAULT_SERVICE_HOST,
            self.config().port(),
            database_name,
            DEFAULT_MONGODB_DIR
        );
        println!("Command : {}", command);
        let _ = Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        // Copy the dump to the host machine
        // docker cp <container_name>:/backup ./backup
        let command = format!(
            "docker cp {}:/{}/{} {}/{}",
            self.config().container_name(),
            DEFAULT_MONGODB_DIR,
            database_name,
            DEFAULT_DATA_DIR,
            database_name
        );
        println!("Command : {}", command);
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        Ok(())
    }

    /// MongoRestore
    /// First copy backup from host to container, then restore
    pub async fn restore_db(&self, database_name: &str) -> Result<(), MongoError> {
        // Copy the backup from host machine to docker container
        // docker cp ./backup <container_name>:/backup
        let command = format!(
            "docker cp {}/{} {}:/{}/{}",
            DEFAULT_DATA_DIR,
            database_name,
            self.config().container_name(),
            DEFAULT_MONGODB_DIR,
            database_name
        );
        println!("Command : {}", command);
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        // Now restore the database inside the container
        // docker exec <container_name> mongorestore --host "localhost:27017" --db orchestrator_3 /backup/database_name
        let command = format!(
            "docker exec {} mongorestore --host \"{}:{}\" --db {} {}/{}",
            self.config().container_name(),
            DEFAULT_SERVICE_HOST,
            self.config().port(),
            database_name,
            DEFAULT_MONGODB_DIR,
            database_name
        );
        println!("Command : {}", command);

        Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        Ok(())
    }
}
