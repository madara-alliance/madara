// =============================================================================
// MADARA SERVICE - Starknet Sequencer using generic Server
// =============================================================================

// Madara will be picked from the binary created, and not the code structure!

use super::util::{MadaraCMD, MadaraConfig, MadaraError, DEFAULT_MADARA_BINARY};
use crate::servers::server::{Server, ServerConfig};
use reqwest::Url;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::time::Duration;

pub struct MadaraService {
    server: Server,
    config: MadaraConfig,
    cmd: MadaraCMD,
}

impl MadaraService {
    /// Start a new Madara service
    pub async fn start(config: MadaraConfig) -> Result<Self, MadaraError> {
        // Validate configuration
        Self::validate_config(&config)?;

        // Create the command
        let cmd = MadaraCMD::from_config(&config);

        // Build the command
        let command = Self::build_command(&config, &cmd)?;

        println!("Starting Madara service with command: {:?}", command);

        // Create server config
        let server_config = ServerConfig {
            port: config.rpc_port,
            connection_attempts: 60, // Madara might take time to start
            connection_delay_ms: 2000,
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await?;

        Ok(Self { server, config, cmd })
    }

    /// Start Madara with a custom command
    pub async fn start_with_cmd(config: MadaraConfig, cmd: MadaraCMD) -> Result<Self, MadaraError> {
        Self::validate_config(&config)?;

        let command = Self::build_command(&config, &cmd)?;

        let server_config = ServerConfig {
            port: config.rpc_port,
            connection_attempts: 60,
            connection_delay_ms: 2000,
            ..Default::default()
        };

        let server = Server::start_process(command, server_config).await?;

        Ok(Self { server, config, cmd })
    }

    /// Validate the configuration
    fn validate_config(config: &MadaraConfig) -> Result<(), MadaraError> {
        // Check if L1 endpoint is provided
        if config.l1_endpoint.is_empty() {
            return Err(MadaraError::MissingConfig("l1_endpoint is required".to_string()));
        }

        // Validate ports are not the same
        if config.rpc_port == config.gateway_port {
            return Err(MadaraError::InvalidConfig("RPC port and Gateway port cannot be the same".to_string()));
        }

        // Check if base path parent directory exists
        if let Some(parent) = config.database_path.parent() {
            if !parent.exists() {
                return Err(MadaraError::MissingConfig(format!(
                    "Base path parent directory does not exist: {}",
                    parent.display()
                )));
            }
        }

        // Validate chain config exists if specified
        if let Some(ref chain_config) = config.chain_config_path {
            if !chain_config.exists() {
                return Err(MadaraError::MissingConfig(format!(
                    "Chain config file does not exist: {}",
                    chain_config.display()
                )));
            }
        }

        Ok(())
    }

    /// Build the command to run Madara
    fn build_command(config: &MadaraConfig, cmd: &MadaraCMD) -> Result<Command, MadaraError> {
        let mut command = if config.release_mode {
            let mut c = Command::new("cargo");
            c.arg("run").arg("--release").arg("--");
            c
        } else {
            // Use binary directly if available
            if let Some(ref binary_path) = config.binary_path {
                Command::new(binary_path)
            } else {
                // Try to find madara binary in PATH
                if Self::check_madara_binary().is_ok() {
                    Command::new(DEFAULT_MADARA_BINARY)
                } else {
                    // Fallback to cargo run
                    let mut c = Command::new("cargo");
                    c.arg("run").arg("--");
                    c
                }
            }
        };

        // Add all arguments
        command.args(&cmd.args);

        // Add environment variables
        for (key, value) in &cmd.env {
            command.env(key, value);
        }

        Ok(command)
    }

    /// Check if Madara binary is available
    fn check_madara_binary() -> Result<(), MadaraError> {
        Command::new(DEFAULT_MADARA_BINARY)
            .arg("--version")
            .output()
            .map_err(|e| MadaraError::BinaryNotFound(e.to_string()))?;
        Ok(())
    }

    /// Get the dependencies required by Madara
    pub fn dependencies(&self) -> Vec<String> {
        vec![
            "anvil".to_string(), // L1 endpoint dependency
        ]
    }

    /// Validate that all required dependencies are available
    pub fn validate_dependencies(&self) -> Result<(), MadaraError> {
        // Check if we can reach L1 endpoint
        // This is a basic check - you might want more sophisticated validation
        if !self.config.l1_endpoint.starts_with("http") {
            return Err(MadaraError::InvalidConfig("L1 endpoint must be a valid HTTP URL".to_string()));
        }

        Ok(())
    }

    /// Validate if Madara is ready and responsive
    pub async fn validate_connection(&self) -> Result<bool, MadaraError> {
        // Try to connect to the RPC endpoint
        let rpc_addr = format!("{}:{}", self.server.host(), self.server.port());
        match tokio::net::TcpStream::connect(&rpc_addr).await {
            Ok(_) => Ok(true),
            Err(e) => Err(MadaraError::ConnectionFailed(e.to_string())),
        }
    }

    /// Check if Madara gateway is responsive
    pub async fn validate_gateway_connection(&self) -> Result<bool, MadaraError> {
        let gateway_addr = format!("{}:{}", self.server.host(), self.config.gateway_port);
        match tokio::net::TcpStream::connect(&gateway_addr).await {
            Ok(_) => Ok(true),
            Err(e) => Err(MadaraError::ConnectionFailed(e.to_string())),
        }
    }

    /// Get the RPC endpoint URL
    pub fn rpc_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", self.server.host(), self.server.port())).unwrap()
    }

    /// Get the Gateway endpoint URL
    pub fn gateway_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", self.server.host(), self.config.gateway_port)).unwrap()
    }

    /// Get the Feeder Gateway endpoint URL
    pub fn feeder_gateway_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}/feeder_gateway", self.server.host(), self.config.gateway_port)).unwrap()
    }

    /// Get the main endpoint URL (alias for rpc_endpoint)
    pub fn endpoint(&self) -> Url {
        self.rpc_endpoint()
    }

    /// Get the RPC port number
    pub fn port(&self) -> u16 {
        self.server.port()
    }

    /// Get the Gateway port number
    pub fn gateway_port(&self) -> u16 {
        self.config.gateway_port
    }

    /// Get the L1 endpoint
    pub fn l1_endpoint(&self) -> &str {
        &self.config.l1_endpoint
    }

    /// Get the chain name
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Get the base path
    pub fn database_path(&self) -> &PathBuf {
        &self.config.database_path
    }

    /// Get the process ID
    pub fn pid(&self) -> Option<u32> {
        self.server.pid()
    }

    /// Check if the process has exited
    pub fn has_exited(&mut self) -> Option<ExitStatus> {
        self.server.has_exited()
    }

    /// Check if the service is running
    pub fn is_running(&mut self) -> bool {
        self.server.is_running()
    }

    /// Stop the Madara service
    pub fn stop(&mut self) -> Result<(), MadaraError> {
        self.server.stop().map_err(MadaraError::Server)
    }

    /// Restart the Madara service (useful after bootstrapper setup)
    pub async fn restart(&mut self) -> Result<(), MadaraError> {
        println!("🔄 Restarting Madara service...");

        // Stop current instance
        self.stop()?;

        // Wait a moment for clean shutdown
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Build new command
        let command = Self::build_command(&self.config, &self.cmd)?;

        // Create server config
        let server_config = ServerConfig {
            port: self.config.rpc_port,
            connection_attempts: 60,
            connection_delay_ms: 2000,
            ..Default::default()
        };

        // Start new instance
        self.server = Server::start_process(command, server_config).await?;

        println!("✅ Madara service restarted");
        Ok(())
    }

    /// Create a configuration for development with Anvil
    pub fn devnet_config(anvil_port: u16) -> MadaraConfig {
        MadaraConfig {
            name: "madara-devnet".to_string(),
            l1_endpoint: format!("http://127.0.0.1:{}", anvil_port),
            chain_config_path: Some(PathBuf::from("configs/presets/devnet.yaml")),
            database_path: PathBuf::from("./madara-db-devnet"),
            ..Default::default()
        }
    }

    /// Create a configuration for testnet
    pub fn testnet_config(l1_endpoint: String) -> MadaraConfig {
        MadaraConfig {
            name: "madara-testnet".to_string(),
            l1_endpoint,
            chain_config_path: Some(PathBuf::from("configs/presets/testnet.yaml")),
            database_path: PathBuf::from("./madara-db-testnet"),
            sequencer: true,
            ..Default::default()
        }
    }

    /// Create a configuration for mainnet
    pub fn mainnet_config(l1_endpoint: String) -> MadaraConfig {
        MadaraConfig {
            name: "madara-mainnet".to_string(),
            l1_endpoint,
            chain_config_path: Some(PathBuf::from("configs/presets/mainnet.yaml")),
            database_path: PathBuf::from("./madara-db-mainnet"),
            sequencer: false, // Usually not a sequencer on mainnet
            gas_price: 100,   // Non-zero for mainnet
            strk_gas_price: 100,
            ..Default::default()
        }
    }

    /// Create a configuration for L3 setup
    pub fn l3_config(l2_endpoint: String) -> MadaraConfig {
        MadaraConfig {
            name: "madara-l3".to_string(),
            l1_endpoint: l2_endpoint, // L3 settles on L2
            chain_config_path: Some(PathBuf::from("configs/presets/l3.yaml")),
            database_path: PathBuf::from("./madara-db-l3"),
            rpc_port: 9945, // Different port for L3
            gateway_port: 8081,
            ..Default::default()
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &MadaraConfig {
        &self.config
    }

    /// Get the current command
    pub fn cmd(&self) -> &MadaraCMD {
        &self.cmd
    }

    /// Create database directory if it doesn't exist
    pub async fn ensure_database_directory(&self) -> Result<(), MadaraError> {
        if !self.config.database_path.exists() {
            tokio::fs::create_dir_all(&self.config.database_path).await?;
            println!("📁 Created database directory: {}", self.config.database_path.display());
        }
        Ok(())
    }

    /// Check if database has been initialized
    pub fn is_database_initialized(&self) -> bool {
        self.config.database_path.exists() && self.config.database_path.join("db").exists()
    }

    /// Get database size in bytes
    pub async fn get_database_size(&self) -> Result<u64, MadaraError> {
        if !self.config.database_path.exists() {
            return Ok(0);
        }

        let mut size = 0u64;
        let mut entries = tokio::fs::read_dir(&self.config.database_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            if metadata.is_file() {
                size += metadata.len();
            }
        }

        Ok(size)
    }
}
