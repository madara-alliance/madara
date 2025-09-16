// =============================================================================
// MONGODB SERVICE - Using Docker and generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
use crate::services::constants::*;
use crate::services::docker::{DockerError, DockerServer};
use crate::services::helpers::get_file_path;
use crate::services::server::{Server, ServerConfig};
pub use config::*;
use reqwest::Url;

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
        if DockerServer::is_port_in_use(config.port()).await {
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
        self.server.stop().map_err(MongoError::Server)?;
        Ok(())
    }
}

// MongoDump and MongoRestore impl from within the docker container
impl MongoService {
    /// Creates a backup of the specified MongoDB database.
    ///
    /// Executes `mongodump` inside the Docker container to create a backup in `/tmp`,
    /// then copies the dump to the host machine at the specified path.
    ///
    /// # Arguments
    /// * `database_path` - Host directory path where the backup will be stored
    /// * `database_name` - Name of the database to backup
    pub async fn dump_db(&self, database_path: &str, database_name: &str) -> Result<(), MongoError> {
        // Step 1: Dump database inside container to /tmp
        let command = format!(
            "docker exec {} mongodump --host \"{}:{}\" --db {} --out /tmp",
            self.config().container_name(),
            DEFAULT_SERVICE_HOST,
            MONGODB_PORT,
            database_name
        );
        let _ = Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        // Get system's db storage directory
        let database_dir_path = get_file_path(database_path).to_string_lossy().into_owned();

        // Step 2: Copy the dump from container to host machine
        // docker cp <container_name>:/tmp/database_name <host_path>
        let command = format!(
            "docker cp {}:/tmp/{} {}",
            self.config().container_name(),
            database_name,
            database_dir_path.as_str(),
        );
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        Ok(())
    }

    /// Restores a MongoDB database from a backup.
    ///
    /// Copies backup files from the host machine to the Docker container's `/tmp` directory,
    /// then executes `mongorestore` to restore the database.
    ///
    /// # Arguments
    /// * `database_path` - Host directory path where the backup is stored
    /// * `database_name` - Name of the database to restore
    pub async fn restore_db(&self, database_path: &str, database_name: &str) -> Result<(), MongoError> {
        // Get system's db storage directory
        let database_dir_path = get_file_path(database_path).to_string_lossy().into_owned();

        // Step 1: Copy the backup from host machine to docker container
        // docker cp <host_path>/database_name <container_name>:/tmp
        let command =
            format!("docker cp {}/{} {}:/tmp", database_dir_path, database_name, self.config().container_name(),);

        Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        // Step 2: Restore the database inside the container using --dir flag
        // docker exec <container_name> mongorestore --host "localhost:27017" --db database_name --dir /tmp/database_name
        let command = format!(
            "docker exec {} mongorestore --host \"{}:{}\" --db {} --dir /tmp/{}",
            self.config().container_name(),
            DEFAULT_SERVICE_HOST,
            MONGODB_PORT,
            database_name,
            database_name
        );

        Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .await
            .map_err(|e| MongoError::Docker(DockerError::Exec(e.to_string())))?;

        Ok(())
    }
}
