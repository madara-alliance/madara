use crate::contract_clients::utils::BroadcastedDeclareTransactionV0;
use futures::{StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult, StarknetError,
};
use starknet_providers::{
    jsonrpc::{
        HttpTransport, HttpTransportError, JsonRpcClientError, JsonRpcMethod, JsonRpcResponse, JsonRpcTransport,
    },
    ProviderError, ProviderRequestData,
};
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum JsonRpcAdminMethod {
    #[serde(rename = "madara_addDeclareV0Transaction")]
    AddDeclareV0Transaction,
    #[serde(rename = "madara_bypassAddDeclareTransaction")]
    BypassAddDeclareTransaction,
    #[serde(rename = "madara_bypassAddDeployAccountTransaction")]
    BypassAddDeployAccountTransaction,
    #[serde(rename = "madara_bypassAddInvokeTransaction")]
    BypassAddInvokeTransaction,
    #[serde(rename = "madara_closeBlock")]
    CloseBlock,
}

#[derive(Debug, Serialize)]
struct JsonRpcAdminRequest<T> {
    id: u64,
    jsonrpc: &'static str,
    method: JsonRpcAdminMethod,
    params: T,
}

pub struct AdminRPCBypassTransport {
    pub inner: HttpTransport,
    pub admin: AdminRPCProvider,
}

#[derive(Clone, Debug)]
pub struct AdminRPCProvider {
    client: Client,
    url: Url,
    headers: Vec<(String, String)>,
}

impl AdminRPCProvider {
    pub fn new(url: impl Into<Url>) -> Self {
        Self::new_with_client(url, Client::new())
    }
    pub fn new_with_client(url: impl Into<Url>, client: Client) -> Self {
        Self { client, url: url.into(), headers: vec![] }
    }
    pub fn with_header(self, name: String, value: String) -> Self {
        let mut headers = self.headers;
        headers.push((name, value));

        Self { client: self.client, url: self.url, headers }
    }
    /// Adds a custom HTTP header to be sent for requests.
    pub fn add_header(&mut self, name: String, value: String) {
        self.headers.push((name, value))
    }

    async fn send_http_request<P: Serialize + Send + Sync, R: DeserializeOwned>(
        &self,
        method: JsonRpcAdminMethod,
        params: P,
    ) -> Result<JsonRpcResponse<R>, HttpTransportError> {
        let request_body = JsonRpcAdminRequest { id: 1, jsonrpc: "2.0", method, params };

        let request_body = serde_json::to_string(&request_body).map_err(HttpTransportError::Json)?;
        log::trace!("Sending request via JSON-RPC Admin: {}", request_body);

        let mut request =
            self.client.post(self.url.clone()).body(request_body).header("Content-Type", "application/json");
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(HttpTransportError::Reqwest)?;

        let response_body = response.text().await.map_err(HttpTransportError::Reqwest)?;
        log::trace!("Response from JSON-RPC Admin: {}", response_body);

        let parsed_response = serde_json::from_str(&response_body).map_err(HttpTransportError::Json)?;

        Ok(parsed_response)
    }

    pub async fn send_request<P: Serialize + Send + Sync, R: DeserializeOwned>(
        &self,
        method: JsonRpcAdminMethod,
        params: P,
    ) -> Result<R, ProviderError> {
        match self.send_http_request(method, params).await.map_err(JsonRpcClientError::TransportError)? {
            JsonRpcResponse::Success { result, .. } => Ok(result),
            JsonRpcResponse::Error { error, .. } => Err(match TryInto::<StarknetError>::try_into(&error) {
                Ok(error) => ProviderError::StarknetError(error),
                Err(_) => JsonRpcClientError::<HttpTransportError>::JsonRpcError(error).into(),
            }),
        }
    }

    pub async fn add_declare_v0_transaction(
        &self,
        tx: BroadcastedDeclareTransactionV0,
    ) -> Result<DeclareTransactionResult, ProviderError> {
        self.send_request(JsonRpcAdminMethod::AddDeclareV0Transaction, [tx]).await
    }

    pub async fn bypass_add_declare_transaction(
        &self,
        tx: BroadcastedDeclareTransaction,
    ) -> Result<DeclareTransactionResult, ProviderError> {
        self.send_request(JsonRpcAdminMethod::BypassAddDeclareTransaction, [tx]).await
    }

    pub async fn bypass_add_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTransaction,
    ) -> Result<DeployAccountTransactionResult, ProviderError> {
        self.send_request(JsonRpcAdminMethod::BypassAddDeployAccountTransaction, [tx]).await
    }

    pub async fn bypass_add_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTransaction,
    ) -> Result<InvokeTransactionResult, ProviderError> {
        self.send_request(JsonRpcAdminMethod::BypassAddInvokeTransaction, [tx]).await
    }

    pub async fn close_block(&self) -> Result<(), ProviderError> {
        self.send_request::<[(); 0], ()>(JsonRpcAdminMethod::CloseBlock, []).await
    }
}

#[async_trait::async_trait]
impl JsonRpcTransport for AdminRPCBypassTransport {
    type Error = HttpTransportError;

    async fn send_request<P: Serialize + Send + Sync, R: DeserializeOwned>(
        &self,
        method: JsonRpcMethod,
        params: P,
    ) -> Result<JsonRpcResponse<R>, HttpTransportError> {
        match method {
            JsonRpcMethod::AddDeclareTransaction => {
                self.admin.send_http_request(JsonRpcAdminMethod::BypassAddDeclareTransaction, params).await
            }
            JsonRpcMethod::AddDeployAccountTransaction => {
                self.admin.send_http_request(JsonRpcAdminMethod::BypassAddDeployAccountTransaction, params).await
            }
            JsonRpcMethod::AddInvokeTransaction => {
                self.admin.send_http_request(JsonRpcAdminMethod::BypassAddInvokeTransaction, params).await
            }
            _ => self.inner.send_request(method, params).await,
        }
    }

    async fn send_requests<R: AsRef<[ProviderRequestData]> + Send + Sync>(
        &self,
        requests: R,
    ) -> Result<Vec<JsonRpcResponse<serde_json::Value>>, HttpTransportError> {
        futures::stream::iter(requests.as_ref())
            .then(|request| async move {
                match request {
                    ProviderRequestData::AddDeclareTransaction(request) => {
                        self.admin.send_http_request(JsonRpcAdminMethod::BypassAddDeclareTransaction, request).await
                    }
                    ProviderRequestData::AddDeployAccountTransaction(request) => {
                        self.admin
                            .send_http_request(JsonRpcAdminMethod::BypassAddDeployAccountTransaction, request)
                            .await
                    }
                    ProviderRequestData::AddInvokeTransaction(request) => {
                        self.admin.send_http_request(JsonRpcAdminMethod::BypassAddInvokeTransaction, request).await
                    }
                    _ => self.inner.send_requests(&[request.clone()]).await.map(|el| el.into_iter().next().unwrap()),
                }
            })
            .try_collect()
            .await
    }
}
