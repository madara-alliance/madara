// =============================================================================
// ORCHESTRATOR SERVICE - Using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use reqwest::Url;
use std::process::ExitStatus;
use std::time::Duration;

pub struct OrchestratorService {
    server: Server, // None for setup mode
    config: OrchestratorConfig,
}

impl OrchestratorService {
    pub async fn run(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        // TODO: config mode should only be run

        let command = config.to_command();

        println!("Running orchestrator in run mode with command : {:?}", command);

        let port = config.port().unwrap();

        // Create server config
        let server_config = ServerConfig {
            rpc_port: Some(port),
            service_name: format!("Orchestrator-{}", config.mode()),
            connection_attempts: 60, // Orchestrator might take time to start
            connection_delay_ms: 2000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(OrchestratorError::Server)?;

        Ok(Self { server, config })
    }

    pub async fn setup(config: OrchestratorConfig) -> Result<ExitStatus, OrchestratorError> {
        // TODO: config mode should only be setup

        let mut service = Self::start_setup_mode(config).await?;
        service.wait_for_completion().await
    }

    /// Wait for the bootstrapper to complete execution
    pub async fn wait_for_completion(&mut self) -> Result<ExitStatus, OrchestratorError> {
        println!("🚀 Running orchestrator in {} mode...", self.config.mode());

        // Use timeout to prevent hanging
        let result = tokio::time::timeout(Duration::from_secs(360), async {
            // Keep checking if the process has exited
            loop {
                if let Some(exit_status) = self.server.has_exited()? {
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
                    println!("✅ Orchestrator {} completed successfully with {}", self.config.mode(), exit_status);
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
    async fn start_setup_mode(config: OrchestratorConfig) -> Result<Self, OrchestratorError> {
        let command = config.to_command();

        println!("Running orchestrator in setup mode with command : {:?}", command);

        let server_config =
            ServerConfig { service_name: format!("Orchestrator-{}", config.mode()), ..Default::default() };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(OrchestratorError::Server)?;

        Ok(Self { server, config })
    }

    pub fn endpoint(&self) -> Option<Url> {
        self.server().endpoint()
    }

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

    pub fn stop(&mut self) -> Result<(), OrchestratorError> {
        println!("☠️ Stopping Orchestrator");
        self.server.stop().map_err(OrchestratorError::Server)?;
        Ok(())
    }

    /// Get the underlying server (run mode only)
    pub fn server(&self) -> &Server {
        &self.server
    }
}

impl OrchestratorService {
    /// Check if State Update for specified block completed or not
    ///
    /// This method calls the orchestrator's job status endpoint to check if the StateTransition
    /// job for the given block number has completed.
    ///
    /// Returns:
    /// - `Ok(true)` if the StateTransition job for the block is completed
    /// - `Ok(false)` if the StateTransition job is not completed or doesn't exist
    /// - `Err(OrchestratorError)` for RPC errors or parsing failures
    pub async fn check_state_update(&self, block_number: u64) -> Result<bool, OrchestratorError> {
        let url = format!("{}jobs/block/{}/status", self.endpoint().unwrap(), block_number);
        let client = reqwest::Client::new();

        println!("Orchestrator URL: {}", url);

        let response = client.get(&url).header("accept", "application/json").send().await.map_err(|e| {
            println!("Failed to send request to orchestrator: {}", e);
            OrchestratorError::NetworkError(e.to_string())
        })?;
        let response_clone = client.get(&url).header("accept", "application/json").send().await.map_err(|e| {
            println!("Failed to send request to orchestrator: {}", e);
            OrchestratorError::NetworkError(e.to_string())
        })?;

        println!("Orchestrator URL: {:?}", response_clone.text().await);

        let json = response.json::<serde_json::Value>().await.map_err(|e| {
            println!("Failed to parse JSON response: {}", e);
            OrchestratorError::InvalidResponse(e.to_string())
        })?;

        println!("JSON RESPONSE : {}", json);

        // Check if the API call was successful
        let success = json.get("success").and_then(|v| v.as_bool()).unwrap_or(false);

        if !success {
            let message = json.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            println!("Orchestrator API error: {}", message);
            return Err(OrchestratorError::InvalidResponse(message.to_string()));
        }

        let empty_vec = vec![];

        // Extract jobs array from the response
        let jobs =
            json.get("data").and_then(|data| data.get("jobs")).and_then(|jobs| jobs.as_array()).unwrap_or(&empty_vec);

        // Look for StateTransition job and check its status
        for job in jobs {
            if let (Some(job_type), Some(status)) =
                (job.get("job_type").and_then(|v| v.as_str()), job.get("status").and_then(|v| v.as_str()))
            {
                if job_type == "StateTransition" {
                    let is_completed = status == "Completed";
                    println!("StateTransition job for block {}: {}", block_number, status);
                    return Ok(is_completed);
                }
            }
        }

        // No StateTransition job found for this block
        println!("No StateTransition job found for block {}", block_number);
        Ok(false)
    }
}
