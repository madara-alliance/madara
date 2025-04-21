use super::builder::PausedClient;
use bytes::{Buf, Bytes};
use http::Method;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use hyper::{HeaderMap, Request, Response, StatusCode, Uri};
use mp_block::{BlockId, BlockTag};
use mp_gateway::error::{SequencerError, StarknetError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use starknet_types_core::felt::Felt;
use std::{borrow::Cow, collections::HashMap};
use tower::Service;
use url::Url;

pub(crate) fn url_join_segment(url: &mut Url, segment: &str) {
    if url.path_segments().expect("Invalid base URL").last().is_some_and(|e| e.is_empty()) {
        url.path_segments_mut().expect("Invalid base URL").pop();
    }
    url.path_segments_mut().expect("Invalid base URL").extend(&[segment]);
}

#[derive(Debug)]
pub struct RequestBuilder<'a> {
    client: &'a PausedClient,
    url: Url,
    params: HashMap<Cow<'static, str>, String>,
    headers: HeaderMap,
}

impl<'a> RequestBuilder<'a> {
    pub fn new(client: &'a PausedClient, base_url: Url, headers: HeaderMap) -> Self {
        Self { client, url: base_url, params: HashMap::new(), headers }
    }

    pub fn add_uri_segment(mut self, segment: &str) -> Result<Self, url::ParseError> {
        url_join_segment(&mut self.url, segment);
        Ok(self)
    }

    #[allow(dead_code)]
    pub fn add_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn add_param(mut self, name: impl Into<Cow<'static, str>>, value: impl Into<Cow<'static, str>>) -> Self {
        self.params.insert(name.into(), value.into().to_string());
        self
    }

    pub fn with_block_id(mut self, block_id: &BlockId) -> Self {
        match block_id {
            BlockId::Hash(hash) => {
                self = self.add_param(Cow::from("blockHash"), format!("0x{hash:x}"));
            }
            BlockId::Number(number) => {
                self = self.add_param(Cow::from("blockNumber"), number.to_string());
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
        self = self.add_param(Cow::from("classHash"), format!("0x{class_hash:x}"));
        self
    }

    pub async fn send_get<T>(self) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
    {
        unpack(self.send_get_raw().await?).await
    }

    pub async fn send_get_raw(self) -> Result<Response<Incoming>, SequencerError> {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method(Method::GET).uri(uri);

        req_builder.headers_mut().expect("Failed to get mutable reference to request headers").extend(self.headers);

        let req = req_builder.body(Full::new(Bytes::from(String::new())))?;

        let response: Response<Incoming> =
            self.client.clone().call(req).await.map_err(SequencerError::HttpCallError)?;
        Ok(response)
    }

    #[cfg(feature = "add_validated_transaction")]
    pub async fn send_post_bincode<T, D>(self, body: D) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
        D: Serialize,
    {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method(Method::POST).uri(uri);

        req_builder.headers_mut().expect("Failed to get mutable reference to request headers").extend(self.headers);

        let body = bincode::serialize(&body).map_err(|err| SequencerError::HttpCallError(err))?;
        let body = Bytes::from(body);

        let req = req_builder.body(Full::new(body))?;

        let response = self.client.clone().call(req).await.map_err(SequencerError::HttpCallError)?;

        let http_status = response.status();
        let whole_body = response.collect().await?.aggregate();

        if http_status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SequencerError::StarknetError(StarknetError::rate_limited()));
        } else if !http_status.is_success() {
            let starknet_error = serde_json::from_reader::<_, StarknetError>(whole_body.reader())
                .map_err(|serde_error| SequencerError::InvalidStarknetError { http_status, serde_error })?;

            return Err(starknet_error.into());
        }

        let res = bincode::deserialize_from(whole_body.reader()).map_err(|err| SequencerError::HttpCallError(err))?;

        Ok(res)
    }

    pub async fn send_post<T, D>(self, body: D) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
        D: Serialize,
    {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method(Method::POST).uri(uri);

        req_builder.headers_mut().expect("Failed to get mutable reference to request headers").extend(self.headers);

        let body = serde_json::to_string(&body).map_err(SequencerError::SerializeRequest)?;

        let req = req_builder.header(CONTENT_TYPE, "application/json").body(Full::new(Bytes::from(body)))?;

        let response = self.client.clone().call(req).await.map_err(SequencerError::HttpCallError)?;
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

async fn unpack<T>(response: Response<Incoming>) -> Result<T, SequencerError>
where
    T: ::serde::de::DeserializeOwned,
{
    let http_status = response.status();
    let whole_body = response.collect().await?.aggregate();

    if http_status == StatusCode::TOO_MANY_REQUESTS {
        return Err(SequencerError::StarknetError(StarknetError::rate_limited()));
    } else if !http_status.is_success() {
        let starknet_error = serde_json::from_reader::<_, StarknetError>(whole_body.reader())
            .map_err(|serde_error| SequencerError::InvalidStarknetError { http_status, serde_error })?;

        return Err(starknet_error.into());
    }

    let res = serde_json::from_reader(whole_body.reader())
        .map_err(|serde_error| SequencerError::DeserializeBody { serde_error })?;

    Ok(res)
}
