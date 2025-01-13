use std::collections::HashMap;

use httpmock::MockServer;
use reqwest::Client;
use serde_json::Value;
use utils::env_utils::get_env_var_or_panic;

#[allow(dead_code)]
/// MockServerGlobal (has mock server inside)
pub struct MockServerGlobal {
    pub(crate) client_url: String,
    pub(crate) mock_server: MockServer,
    pub(crate) client: Option<Client>,
}

pub enum MockResponseBodyType {
    Json(Value),
    String(String),
}

impl MockServerGlobal {
    /// To create a new client
    pub fn new() -> Self {
        let server = MockServer::start();
        Self { client_url: format!("http://localhost:{:?}", server.port()), mock_server: server, client: None }
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
        response_body: MockResponseBodyType,
    ) {
        self.mock_server.mock(|when, then| {
            let mut request = when.path(path);

            for condition in body_contains {
                request = request.body_includes(&condition);
            }

            match response_body {
                MockResponseBodyType::Json(val) => {
                    then.status(status.unwrap_or(200)).body(serde_json::to_vec(&val).unwrap());
                }
                MockResponseBodyType::String(val) => {
                    then.status(status.unwrap_or(200)).body(val);
                }
            }
        });
    }

    pub fn connect(rpc_url: String) -> Self {
        Self { client_url: rpc_url.clone(), mock_server: MockServer::connect(&rpc_url), client: None }
    }

    pub fn proxy(rpc_url: String, method_response_hashmap: HashMap<String, Value>) -> Self {
        let (mock, client) = Self::start_proxy_server(&rpc_url, method_response_hashmap);
        Self { client_url: rpc_url.clone(), mock_server: mock, client: Some(client) }
    }

    fn start_proxy_server(_target_url: &str, method_response_hashmap: HashMap<String, Value>) -> (MockServer, Client) {
        let proxy_server = MockServer::start();

        let client = Client::builder().proxy(reqwest::Proxy::all(proxy_server.base_url()).unwrap()).build().unwrap();

        for (req_path, return_val) in method_response_hashmap {
            proxy_server.mock(|when, then| {
                when.body_includes(req_path.clone());
                then.json_body(return_val);
            });

            let snos_url = get_env_var_or_panic("MADARA_ORCHESTRATOR_RPC_FOR_SNOS");
            let snos_host = snos_url.split("://").last().unwrap().split(":").next().unwrap();
            let snos_port = snos_url.split("://").last().unwrap().split(":").last().unwrap();
            proxy_server.proxy(|rule| {
                rule.filter(|when| {
                    when.host(snos_host).port(snos_port.parse::<u16>().unwrap()).body_excludes(req_path.clone());
                });
            });
        }

        (proxy_server, client)
    }
}

impl Default for MockServerGlobal {
    fn default() -> Self {
        Self::new()
    }
}
