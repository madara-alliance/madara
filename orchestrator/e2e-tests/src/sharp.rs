use httpmock::MockServer;

use crate::mock_server::{MockResponseBodyType, MockServerGlobal};

/// Starknet Client struct (has mock server inside)
pub struct SharpClient {
    client: MockServerGlobal,
}

impl SharpClient {
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
        response_body: MockResponseBodyType,
    ) {
        self.client.add_mock_on_endpoint(path, body_contains, status, response_body);
    }
}

impl Default for SharpClient {
    fn default() -> Self {
        Self::new()
    }
}
