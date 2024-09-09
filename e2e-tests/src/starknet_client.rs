use crate::mock_server::MockServerGlobal;
use httpmock::MockServer;
use serde_json::Value;

/// Starknet Client struct (has mock server inside)
pub struct StarknetClient {
    client: MockServerGlobal,
}

impl StarknetClient {
    /// To create a new client
    pub fn new() -> Self {
        Self { client: MockServerGlobal::new() }
    }

    /// To get mutable mock server ref for adding expects for URLs
    pub fn mut_mock_server(&mut self) -> &mut MockServer {
        &mut self.client.mock_server
    }

    /// To get the server URL
    pub fn url(&self) -> String {
        self.client.client_url.clone()
    }

    /// To add mock on the mock server endpoints
    pub fn add_mock_on_endpoint(
        &mut self,
        path: &str,
        body_contains: Vec<String>,
        status: Option<u16>,
        response_body: &Value,
    ) {
        self.client.add_mock_on_endpoint(path, body_contains, status, response_body);
    }
}

impl Default for StarknetClient {
    fn default() -> Self {
        Self::new()
    }
}
