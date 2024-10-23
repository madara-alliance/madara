use std::{borrow::Cow, collections::HashMap};

use hyper::client::HttpConnector;
use hyper::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use hyper::{Body, Client};
use hyper::{HeaderMap, Request, Response, StatusCode, Uri};
use hyper_tls::HttpsConnector;
use mp_block::{BlockId, BlockTag};
use serde::de::DeserializeOwned;
use starknet_types_core::felt::Felt;
use tower::{retry::Retry, timeout::Timeout};
use url::Url;

use crate::error::{SequencerError, StarknetError};

use super::builder::{PauseLayerMiddleware, RetryPolicy};

#[derive(Debug)]
pub struct RequestBuilder<'a> {
    client: &'a PauseLayerMiddleware<Retry<RetryPolicy, Timeout<Client<HttpsConnector<HttpConnector>, Body>>>>,
    url: Url,
    params: HashMap<Cow<'static, str>, String>,
    headers: HeaderMap,
}

impl<'a> RequestBuilder<'a> {
    pub fn new(
        client: &'a PauseLayerMiddleware<Retry<RetryPolicy, Timeout<Client<HttpsConnector<HttpConnector>, Body>>>>,
        base_url: Url,
        headers: HeaderMap,
    ) -> Self {
        Self { client, url: base_url, params: HashMap::new(), headers }
    }

    pub fn add_uri_segment(mut self, segment: &str) -> Result<Self, url::ParseError> {
        self.url = self.url.join(segment)?;
        Ok(self)
    }

    #[allow(dead_code)]
    pub fn add_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn add_param(mut self, name: Cow<'static, str>, value: &str) -> Self {
        self.params.insert(name, value.to_string());
        self
    }

    pub fn with_block_id(mut self, block_id: BlockId) -> Self {
        match block_id {
            BlockId::Hash(hash) => {
                self = self.add_param(Cow::from("blockHash"), &format!("0x{hash:x}"));
            }
            BlockId::Number(number) => {
                self = self.add_param(Cow::from("blockNumber"), &number.to_string());
            }
            BlockId::Tag(tag) => {
                let tag = match tag {
                    BlockTag::Latest => "latest",
                    BlockTag::Pending => "pending",
                };
                self = self.add_param(Cow::from("blockNumber"), tag);
            }
        }
        self
    }

    pub fn with_class_hash(mut self, class_hash: Felt) -> Self {
        self = self.add_param(Cow::from("classHash"), &format!("0x{class_hash:x}"));
        self
    }

    pub async fn send_get<T>(self) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
    {
        unpack(self.send_get_raw().await?).await
    }

    pub async fn send_get_raw(self) -> Result<Response<Body>, SequencerError> {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method("GET").uri(uri);

        for (key, value) in self.headers.iter() {
            req_builder = req_builder.header(key, value);
        }

        let req = req_builder.body(Body::empty()).unwrap();

        let response = self.client.clone().call(req).await?;
        Ok(response)
    }

    #[allow(dead_code)]
    pub async fn send_post<T>(self) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
    {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method("POST").uri(uri);

        for (key, value) in self.headers.iter() {
            req_builder = req_builder.header(key, value);
        }

        let body = serde_json::to_string(&self.params)?;

        let req = req_builder.header(CONTENT_TYPE, "application/json").body(Body::from(body))?;

        let response = self.client.clone().call(req).await?;
        unpack(response).await
    }

    fn build_uri(&self) -> Result<Uri, SequencerError> {
        let mut url = self.url.clone();
        let query: String =
            self.params.iter().map(|(key, value)| format!("{}={}", key, value)).collect::<Vec<String>>().join("&");

        if !query.is_empty() {
            url.set_query(Some(&query));
        }

        let uri: Uri = url.as_str().try_into().map_err(|_| SequencerError::InvalidUrl(url))?;
        Ok(uri)
    }
}

async fn unpack<T>(response: Response<Body>) -> Result<T, SequencerError>
where
    T: ::serde::de::DeserializeOwned,
{
    let http_status = response.status();
    if http_status == StatusCode::TOO_MANY_REQUESTS {
        return Err(SequencerError::StarknetError(StarknetError::rate_limited()));
    } else if !http_status.is_success() {
        let body = hyper::body::to_bytes(response.into_body()).await?;
        let starknet_error = serde_json::from_slice::<StarknetError>(&body)
            .map_err(|serde_error| SequencerError::InvalidStarknetError { http_status, serde_error, body })?;

        return Err(starknet_error.into());
    }

    let body = hyper::body::to_bytes(response.into_body()).await?;
    let res =
        serde_json::from_slice(&body).map_err(|serde_error| SequencerError::DeserializeBody { serde_error, body })?;

    Ok(res)
}
