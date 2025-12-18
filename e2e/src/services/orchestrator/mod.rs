// =============================================================================
// ORCHESTRATOR SERVICE - Using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use reqwest::Url;
use serde::Deserialize;
use std::process::ExitStatus;
use std::time::Duration;

// JSON response types
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: T,
    message: String,
}

#[derive(Debug, Deserialize)]
struct JobsData {
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
#[allow(unused)]
struct Job {
    job_type: String,
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct BatchData {
    batch_number: u64,
}

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
    /// job for the given block number has completed. It also verifies that no jobs are left in
    /// Created or LockedForProcessing state.
    ///
    /// Returns:
    /// - `Ok(true)` if the StateTransition job for the block is completed AND no jobs are pending
    /// - `Ok(false)` if the StateTransition job is not completed, doesn't exist, or jobs are still pending
    /// - `Err(OrchestratorError)` for RPC errors or parsing failures
    pub async fn check_state_update(&self, block_number: u64) -> Result<bool, OrchestratorError> {
        let client = reqwest::Client::new();
        let endpoint = self.endpoint().unwrap();

        // Step 1: Check for jobs in Created state
        let url_get_created = format!("{}jobs?status=Created", endpoint);
        let response_created =
            client.get(&url_get_created).header("accept", "application/json").send().await.map_err(|e| {
                println!("Failed to fetch Created jobs: {}", e);
                OrchestratorError::NetworkError(e.to_string())
            })?;

        let created_jobs: ApiResponse<JobsData> = response_created.json().await.map_err(|e| {
            println!("Failed to parse Created jobs response: {}", e);
            OrchestratorError::InvalidResponse(e.to_string())
        })?;

        if !created_jobs.success {
            println!("Orchestrator API error (Created jobs): {}", created_jobs.message);
            return Err(OrchestratorError::InvalidResponse(created_jobs.message));
        }

        if !created_jobs.data.jobs.is_empty() {
            println!("Found {} jobs in Created state, waiting...", created_jobs.data.jobs.len());
            return Ok(false);
        }

        // Step 2: Check for jobs in LockedForProcessing state
        let url_get_locked = format!("{}jobs?status=LockedForProcessing", endpoint);
        let response_locked =
            client.get(&url_get_locked).header("accept", "application/json").send().await.map_err(|e| {
                println!("Failed to fetch LockedForProcessing jobs: {}", e);
                OrchestratorError::NetworkError(e.to_string())
            })?;

        let locked_jobs: ApiResponse<JobsData> = response_locked.json().await.map_err(|e| {
            println!("Failed to parse LockedForProcessing jobs response: {}", e);
            OrchestratorError::InvalidResponse(e.to_string())
        })?;

        if !locked_jobs.success {
            println!("Orchestrator API error (LockedForProcessing jobs): {}", locked_jobs.message);
            return Err(OrchestratorError::InvalidResponse(locked_jobs.message));
        }

        if !locked_jobs.data.jobs.is_empty() {
            println!("Found {} jobs in LockedForProcessing state, waiting...", locked_jobs.data.jobs.len());
            return Ok(false);
        }

        // Step 3: Fetch batch for the block
        let url_batch = format!("{}blocks/batch-for-block/{}", endpoint, block_number);
        let response_batch = client.get(&url_batch).header("accept", "application/json").send().await.map_err(|e| {
            println!("Failed to fetch batch for block {}: {}", block_number, e);
            OrchestratorError::NetworkError(e.to_string())
        })?;

        let batch_response: ApiResponse<BatchData> = response_batch.json().await.map_err(|e| {
            println!("Failed to parse batch response: {}", e);
            OrchestratorError::InvalidResponse(e.to_string())
        })?;

        if !batch_response.success {
            println!("Orchestrator API error (batch): {}", batch_response.message);
            return Err(OrchestratorError::InvalidResponse(batch_response.message));
        }

        let batch_number = batch_response.data.batch_number;

        // Step 4: Fetch the status of the batch's state_transition job
        let url_status = format!("{}jobs/block/{}/status", endpoint, batch_number);
        let response_status =
            client.get(&url_status).header("accept", "application/json").send().await.map_err(|e| {
                println!("Failed to fetch job status for batch {}: {}", batch_number, e);
                OrchestratorError::NetworkError(e.to_string())
            })?;

        let status_response: ApiResponse<JobsData> = response_status.json().await.map_err(|e| {
            println!("Failed to parse job status response: {}", e);
            OrchestratorError::InvalidResponse(e.to_string())
        })?;

        if !status_response.success {
            println!("Orchestrator API error (job status): {}", status_response.message);
            return Err(OrchestratorError::InvalidResponse(status_response.message));
        }

        // Step 5: Look for StateTransition job and check its status
        for job in &status_response.data.jobs {
            if job.job_type == "StateTransition" {
                let is_completed = job.status == "Completed";
                println!("StateTransition job for block {} (batch {}): {}", block_number, batch_number, job.status);
                return Ok(is_completed);
            }
        }

        // No StateTransition job found for this block
        println!("No StateTransition job found for block {} (batch {})", block_number, batch_number);
        Ok(false)
    }
}
