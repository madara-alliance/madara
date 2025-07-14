
// For localstack we have to use the localstack rule
// Setup
// cargo run --bin orchestrator setup --layer l2 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type rule
// cargo run --bin orchestrator setup --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type rule

// Run
// cargo run --bin orchestrator run --layer l2 --aws --settle-on-ethereum --aws-s3 --aws-sqs --aws-sns --da-on-ethereum --mongodb --atlantic
// cargo run --bin orchestrator run --layer l3 --aws --settle-on-starknet --aws-s3 --aws-sqs --aws-sns --da-on-starknet --mongodb --atlantic

// =============================================================================
// ORCHESTRATOR SERVICE - Using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::servers::server::ServerError;
use crate::servers::server::{Server, ServerConfig};
use reqwest::Url;
use std::process::ExitStatus;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub struct OrchestratorService {
    server: Server, // None for setup mode
    config: OrchestratorConfig,
}

impl OrchestratorService {
    /// Start the orchestrator service
    pub async fn start(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        println!("Statting Orchestrator  #1");

        match config.mode() {
            OrchestratorMode::Setup => Self::run_setup_mode(config).await,
            OrchestratorMode::Run => Self::run_run_mode(config).await,
        }
    }


    pub async fn setup(config: OrchestratorConfig) -> Result<ExitStatus, OrchestratorError> {
        let mut service = Self::start(config).await?;
        service.wait_for_completion().await
    }


    /// Wait for the bootstrapper to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<ExitStatus, OrchestratorError> {
        println!("🚀 Running bootstrapper in {} mode...", self.config.mode());

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(Duration::from_secs(360), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited() {
                    return Ok::<ExitStatus, OrchestratorError>(exit_status);
                }

                // Small delay to avoid busy waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(Ok(exit_status)) => {
                if exit_status.success() {
                    println!("✅ Bootstrapper {} completed successfully with {}", self.config.mode(), exit_status);
                    Ok(exit_status)
                } else {
                    let exit_code = exit_status.code().unwrap_or(-1);
                    Err(OrchestratorError::SetupFailed(exit_code))
                }
            }
            Ok(Err(e)) => Err(OrchestratorError::ExecutionFailed(e.to_string())),
            Err(_) => Err(OrchestratorError::ExecutionFailed(format!(
                "Orchestrator timed out after {:?}",
                Duration::from_secs(360)
            ))),
        }
    }


    /// Run in setup mode (blocking, returns when complete)
    async fn run_setup_mode(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        let mut command = Self::build_setup_command(&config);

        println!("Running orchestrator in setup mode with command : {:?}", command);

        let server_config = ServerConfig {
            port: 0, // Dummy port - bootstrapper doesn't serve on a port
            host: "127.0.0.1".to_string(),
            skip_wait_for_ready: true,
            connection_attempts: 1, // No connection check needed
            connection_delay_ms: 100,
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(OrchestratorError::Server)?;


        // // For setup mode, we run the command directly and wait for completion
        // let mut child = command.spawn().map_err(|e| OrchestratorError::Server(ServerError::StartupFailed(e)))?;

        // // Wait for the process to complete
        // let status = child.wait().await.map_err(|e| OrchestratorError::Server(ServerError::Io(e)))?;

        // if status.success() {
        //     println!("Orchestrator cloud setup completed ✅");
        //     Ok(Self { server: None, config, address: None })
        // } else {
        //     let exit_code = status.code().unwrap_or(-1);
        //     Err(OrchestratorError::SetupFailed(exit_code))
        // }
        Ok(Self { server, config })
    }

    /// Run in run mode (async, returns immediately with running server)
    async fn run_run_mode(mut config: OrchestratorConfig) -> Result<Self, OrchestratorError> {

        let port = config.port().unwrap();
        let address = format!("127.0.0.1:{}", port);

        // Add port to environment variables
        // Since config is immutable, we need to rebuild it with the port env var
        let mut env_vars = config.environment_vars().to_vec();
        env_vars.push(("MADARA_ORCHESTRATOR_PORT".to_string(), port.to_string()));

        config = OrchestratorConfigBuilder::new()
            .mode(config.mode().clone())
            .layer(config.layer().clone())
            .port(config.port())
            .environment_vars(env_vars)
            .aws(config.aws())
            .aws_s3(config.aws_s3())
            .aws_sqs(config.aws_sqs())
            .aws_sns(config.aws_sns())
            .aws_event_bridge(config.aws_event_bridge())
            .event_bridge_type(config.event_bridge_type().map(|s| s.to_string()))
            .settle_on_ethereum(config.settle_on_ethereum())
            .settle_on_starknet(config.settle_on_starknet())
            .da_on_ethereum(config.da_on_ethereum())
            .da_on_starknet(config.da_on_starknet())
            .sharp(config.sharp())
            .mongodb(config.mongodb())
            .atlantic(config.atlantic())
            .build();

        let command = Self::build_run_command(&config);

        println!("Running orchestrator in run mode on {}", address);

        // Create server config
        let server_config = ServerConfig {
            port,
            host: "127.0.0.1".to_string(),
            connection_attempts: 60, // Orchestrator might take time to start
            connection_delay_ms: 2000,
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(OrchestratorError::Server)?;

        Ok(Self { server, config })
    }

    /// Build command for setup mode
    fn build_setup_command(config: &OrchestratorConfig) -> Command {
        let mut command = Command::new(config.binary_path());

        if *config.mode() == OrchestratorMode::Setup {
            command.arg("setup");
        } else {
            command.arg("run");
        }

        if *config.layer() == Layer::L2 {
            command.arg("--layer").arg("l2");
        } else {
            command.arg("--layer").arg("l3");
        }

        // Add AWS flags
        if config.aws() {
            command.arg("--aws");
        }
        if config.aws_s3() {
            command.arg("--aws-s3");
        }
        if config.aws_sqs() {
            command.arg("--aws-sqs");
        }
        if config.aws_sns() {
            command.arg("--aws-sns");
        }
        if config.aws_event_bridge() {
            command.arg("--aws-event-bridge");
        }
        if let Some(event_bridge_type) = config.event_bridge_type() {
            command.arg("--event-bridge-type").arg(event_bridge_type);
        }

        command.arg("--queue-identifier").arg("test_{}_queue");

        // For setup mode, inherit stdio to show output directly
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());

        // Add environment variables
        for (key, value) in config.environment_vars() {
            command.env(key, value);
        }

        command
    }

    /// Build command for run mode
    fn build_run_command(config: &OrchestratorConfig) -> Command {
        let mut command = Command::new("cargo");
        command
            .arg("run")
            .arg("--release")
            .arg("-p")
            .arg("orchestrator")
            .arg("--features")
            .arg("testing")
            .arg("run")
            .arg(&format!("--layer={}", config.layer()));

        // Add AWS flags
        if config.aws() {
            command.arg("--aws");
        }
        if config.aws_s3() {
            command.arg("--aws-s3");
        }
        if config.aws_sqs() {
            command.arg("--aws-sqs");
        }
        if config.aws_sns() {
            command.arg("--aws-sns");
        }

        // Add settlement and DA options
        if config.settle_on_ethereum() {
            command.arg("--settle-on-ethereum");
        }
        if config.settle_on_starknet() {
            command.arg("--settle-on-starknet");
        }
        if config.da_on_ethereum() {
            command.arg("--da-on-ethereum");
        }
        if config.da_on_starknet() {
            command.arg("--da-on-starknet");
        }
        if config.sharp() {
            command.arg("--sharp");
        }
        if config.mongodb() {
            command.arg("--mongodb");
        }
        if config.atlantic() {
            command.arg("--atlantic");
        }

        // For run mode, pipe stdout and stderr
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        // Add environment variables
        for (key, value) in config.environment_vars() {
            command.env(key, value);
        }


        command
    }

    /// Get the repository root directory
    fn get_repository_root() -> Result<PathBuf, OrchestratorError> {
        // Try to find git repository root
        let mut current_dir = std::env::current_dir().map_err(OrchestratorError::WorkingDirectoryFailed)?;

        loop {
            if current_dir.join(".git").exists() {
                return Ok(current_dir);
            }

            if let Some(parent) = current_dir.parent() {
                current_dir = parent.to_path_buf();
            } else {
                break;
            }
        }

        // Fallback to current directory
        std::env::current_dir().map_err(OrchestratorError::WorkingDirectoryFailed)
    }

    /// Get the dependencies required by the orchestrator
    pub fn dependencies(&self) -> Vec<String> {
        vec![
            // internal
            "anvil".to_string(),
            "madara".to_string(),
            "pathfinder".to_string(),
            // TODO: Actually bootstrapper is not a direct dep of orchestrator
            // we can remove this
            "bootstrapper_l1".to_string(),
            "bootstrapper_l2".to_string(),
            // external
            "atlantic".to_string(),
            "localstack".to_string(),
            "mongodb".to_string(),
        ]
    }

    /// Validate that all required dependencies are available and running
    /// TODO: might move this to a fn in setup
    pub async fn validate_dependencies(&self) -> Result<(), OrchestratorError> {
        // TODO: complete this!
        let dependencies = self.dependencies();

        for dep in dependencies {
            // For now, just check if the command exists
            // You might want to implement more sophisticated checking
            let result = Command::new(&dep).arg("--version").output().await;

            if result.is_err() {
                return Err(OrchestratorError::MissingDependency(dep));
            }
        }

        Ok(())
    }

    /// Create a setup configuration for L2
    pub fn setup_l2_config() -> OrchestratorConfig {
        OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Setup)
            .layer(Layer::L2)
            .aws_event_bridge(true)
            .event_bridge_type(Some("rule"))
            .build()
    }

    /// Create a setup configuration for L3
    pub fn setup_l3_config() -> OrchestratorConfig {
        OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Setup)
            .layer(Layer::L3)
            .aws_event_bridge(true)
            .event_bridge_type(Some("rule"))
            .build()
    }

    /// Create a run configuration for L2
    pub fn run_l2_config() -> OrchestratorConfig {
        OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Run)
            .layer(Layer::L2)
            .settle_on_ethereum(true)
            .da_on_ethereum(true)
            .mongodb(true)
            .atlantic(true)
            .build()
    }

    /// Create a run configuration for L3
    pub fn run_l3_config() -> OrchestratorConfig {
        OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Run)
            .layer(Layer::L3)
            .settle_on_starknet(true)
            .da_on_starknet(true)
            .mongodb(true)
            .atlantic(true)
            .build()
    }

    // /// Get the endpoint URL for the orchestrator service (run mode only)
    // pub fn endpoint(&self) -> Option<Url> {
    //     // TODO: validate run mode is being used
    //     if let Some(ref address) = self.address {
    //         Url::parse(&format!("http://{}", address)).ok()
    //     } else {
    //         None
    //     }
    // }

    // TODO: A mongodb respective fn that dumps and loads the db

    /// Get the current mode
    pub fn mode(&self) -> &OrchestratorMode {
        self.config.mode()
    }

    /// Get the port number (run mode only)
    pub fn port(&self) -> Option<u16> {
        self.config.port()
    }

    /// Get the layer
    pub fn layer(&self) -> &Layer {
        self.config.layer()
    }

    /// Get the configuration used
    pub fn config(&self) -> &OrchestratorConfig {
        &self.config
    }

    /// Get the underlying server (run mode only)
    pub fn server(&self) -> &Server {
        &self.server
    }

    // pub fn server(&self) -> &Server {
    //     &self.server.unwrap()
    // }
}
