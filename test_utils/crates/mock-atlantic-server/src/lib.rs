pub mod server;
pub mod types;

use axum::Router;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::{error, info};

pub use server::{create_router, MockAtlanticState};
pub use types::{AtlanticAddJobResponse, AtlanticGetStatusResponse, MockServerConfig};

/// Mock Atlantic Server
///
/// This server mimics the Atlantic API endpoints for testing purposes.
/// It provides the same HTTP interface as the real Atlantic service.
pub struct MockAtlanticServer {
    router: Router,
    addr: SocketAddr,
}

impl MockAtlanticServer {
    /// Create a new mock Atlantic server instance
    pub fn new(addr: SocketAddr, config: MockServerConfig) -> Self {
        let state = MockAtlanticState::new(config);
        let router = create_router(state);

        Self { router, addr }
    }

    /// Start the mock server
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting Mock Atlantic Server on {}", self.addr);

        let listener = TcpListener::bind(self.addr).await?;

        info!("Mock Atlantic Server listening on http://{}", self.addr);
        info!("Available endpoints:");
        info!("  POST   /atlantic-query        - Submit new job");
        info!("  GET    /atlantic-query/{{id}}  - Get job status");
        info!("  GET    /is-alive                - Is alive check");

        if let Err(e) = axum::serve(listener, self.router).await {
            error!("Server error: {}", e);
            return Err(Box::new(e));
        }

        Ok(())
    }

    /// Create a server with default configuration for testing
    pub fn with_test_config(port: u16) -> Self {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let config = MockServerConfig {
            simulate_failures: false,
            processing_delay_ms: 1000,
            failure_rate: 0.0,
            auto_complete_jobs: true,
            completion_delay_ms: 2000,
            max_jobs_in_memory: 500,
            max_concurrent_jobs: 10,
        };
        Self::new(addr, config)
    }

    /// Create a server with failure simulation enabled
    pub fn with_failure_simulation(port: u16, failure_rate: f32) -> Self {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let config = MockServerConfig {
            simulate_failures: true,
            processing_delay_ms: 500,
            failure_rate,
            auto_complete_jobs: true,
            completion_delay_ms: 1500,
            max_jobs_in_memory: 500,
            max_concurrent_jobs: 5,
        };
        Self::new(addr, config)
    }
}

/// Utility function to run a mock server in the background for testing
pub async fn start_mock_server_background(port: u16) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let server = MockAtlanticServer::with_test_config(port);
        if let Err(e) = server.run().await {
            error!("Mock server failed: {}", e);
        }
    })
}

/// Start the mock Atlantic server with default configuration
/// This function will block until the server is shut down
pub async fn start_mock_atlantic_server() -> Result<(), Box<dyn std::error::Error>> {
    let port = 4001; // Default Atlantic mock server port
    let server = MockAtlanticServer::with_test_config(port);
    server.run().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_mock_server_basic_flow() {
        let port = 8080;
        let _handle = start_mock_server_background(port).await;

        // Give the server time to start
        sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let base_url = format!("http://127.0.0.1:{}", port);

        // Test health check
        let response = client.get(format!("{}/health", base_url)).send().await.expect("Health check failed");

        assert!(response.status().is_success());

        // Test job submission (simplified - real test would use multipart)
        let form = reqwest::multipart::Form::new()
            .text("layout", "dynamic")
            .text("network", "TESTNET")
            .text("declaredJobSize", "S");

        let response = client
            .post(format!("{}/atlantic-query?apiKey=test", base_url))
            .multipart(form)
            .send()
            .await
            .expect("Job submission failed");

        assert!(response.status().is_success());
        let job_response: AtlanticAddJobResponse = response.json().await.expect("Failed to parse response");

        // Test job status
        let response = client
            .get(format!("{}/atlantic-query/{}", base_url, job_response.atlantic_query_id))
            .send()
            .await
            .expect("Status check failed");

        assert!(response.status().is_success());
        let status_response: AtlanticGetStatusResponse = response.json().await.expect("Failed to parse status");
        assert_eq!(status_response.atlantic_query.id, job_response.atlantic_query_id);
    }
}
