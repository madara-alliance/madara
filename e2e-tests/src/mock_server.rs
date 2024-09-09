use httpmock::MockServer;
use serde_json::Value;

/// MockServerGlobal (has mock server inside)
pub struct MockServerGlobal {
    pub(crate) client_url: String,
    pub(crate) mock_server: MockServer,
}

impl MockServerGlobal {
    /// To create a new client
    pub fn new() -> Self {
        let server = MockServer::start();
        Self { client_url: format!("http://localhost:{:?}", server.port()), mock_server: server }
    }

    /// To get mutable mock server ref for adding expects for URLs
    pub fn mut_mock_server(&mut self) -> &mut MockServer {
        &mut self.mock_server
    }

    /// To get the server URL
    pub fn url(&self) -> String {
        self.client_url.clone()
    }

    /// To add mock on the mock server endpoints
    pub fn add_mock_on_endpoint(
        &mut self,
        path: &str,
        body_contains: Vec<String>,
        status: Option<u16>,
        response_body: &Value,
    ) {
        self.mock_server.mock(|when, then| {
            let mut request = when.path(path);

            for condition in body_contains {
                request = request.body_contains(&condition);
            }

            then.status(status.unwrap_or(200)).body(serde_json::to_vec(response_body).unwrap());
        });
    }
}

impl Default for MockServerGlobal {
    fn default() -> Self {
        Self::new()
    }
}
