use super::builder::PausedClient;
use bincode::Options;
use bytes::{Buf, Bytes};
use http::Method;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use hyper::{HeaderMap, Request, Response, StatusCode, Uri};
use mp_gateway::error::{SequencerError, StarknetError};
use mp_rpc::v0_8_1::{BlockId, BlockTag};
use serde::de::DeserializeOwned;
use serde::Serialize;
use starknet_types_core::felt::Felt;
use std::time::{Duration, Instant};
use std::{borrow::Cow, collections::HashMap};
use tower::Service;
use url::Url;

use crate::metrics::{error_kind, metrics, request_labels_from_url, request_result, RequestLabels};

pub(crate) fn url_join_segment(url: &mut Url, segment: &str) {
    if url.path_segments().expect("Invalid base URL").next_back().is_some_and(|e| e.is_empty()) {
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
        let telemetry = self.telemetry("GET");
        let start = Instant::now();
        let response = self.send_request(Method::GET, Bytes::new(), None, &telemetry, start).await?;
        unpack(response, &telemetry, start).await
    }

    pub async fn send_get_raw(self) -> Result<Response<Incoming>, SequencerError> {
        let telemetry = self.telemetry("GET");
        let start = Instant::now();
        let response = self.send_request(Method::GET, Bytes::new(), None, &telemetry, start).await?;
        record_successful_request(&telemetry, response.status(), start.elapsed());
        Ok(response)
    }

    pub async fn send_post_bincode<T, D>(self, body: D) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
        D: Serialize,
    {
        let telemetry = self.telemetry("POST");
        let start = Instant::now();

        let body = bincode::options().with_little_endian().serialize(&body).map_err(|err| {
            let error = SequencerError::HttpCallError(err);
            record_failed_request(&telemetry, None, error_kind::REQUEST_SERIALIZE, start.elapsed(), &error);
            error
        })?; // Fixed endinaness is important.
        let body = Bytes::from(body);

        let response = self.send_request(Method::POST, body, None, &telemetry, start).await?;

        let http_status = response.status();
        let whole_body = response
            .collect()
            .await
            .map_err(|error| {
                let error = SequencerError::HyperError(error);
                record_failed_request(&telemetry, Some(http_status), error_kind::TRANSPORT, start.elapsed(), &error);
                error
            })?
            .aggregate();

        if http_status == StatusCode::TOO_MANY_REQUESTS {
            let error = StarknetError::rate_limited();
            record_starknet_failure(&telemetry, http_status, start.elapsed(), &error);
            return Err(SequencerError::StarknetError(error));
        } else if !http_status.is_success() {
            let starknet_error =
                serde_json::from_reader::<_, StarknetError>(whole_body.reader()).map_err(|serde_error| {
                    let error = SequencerError::InvalidStarknetError { http_status, serde_error };
                    record_failed_request(
                        &telemetry,
                        Some(http_status),
                        error_kind::INVALID_STARKNET,
                        start.elapsed(),
                        &error,
                    );
                    error
                })?;

            record_starknet_failure(&telemetry, http_status, start.elapsed(), &starknet_error);
            return Err(starknet_error.into());
        }

        let res = bincode::options()
            .with_little_endian() // Fixed endinaness is important.
            .deserialize_from(whole_body.reader())
            .map_err(|err| {
                let error = SequencerError::HttpCallError(err);
                record_failed_request(
                    &telemetry,
                    Some(http_status),
                    error_kind::RESPONSE_DESERIALIZE,
                    start.elapsed(),
                    &error,
                );
                error
            })?;

        record_successful_request(&telemetry, http_status, start.elapsed());

        Ok(res)
    }

    pub async fn send_post<T, D>(self, body: D) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
        D: Serialize,
    {
        let telemetry = self.telemetry("POST");
        let start = Instant::now();

        let body = serde_json::to_string(&body).map_err(|serde_error| {
            let error = SequencerError::SerializeRequest(serde_error);
            record_failed_request(&telemetry, None, error_kind::REQUEST_SERIALIZE, start.elapsed(), &error);
            error
        })?;

        let response =
            self.send_request(Method::POST, Bytes::from(body), Some("application/json"), &telemetry, start).await?;
        unpack(response, &telemetry, start).await
    }

    fn telemetry(&self, http_method: &'static str) -> RequestTelemetry {
        let labels = request_labels_from_url(&self.url);
        RequestTelemetry { labels, http_method }
    }

    async fn send_request(
        self,
        http_method: Method,
        body: Bytes,
        content_type: Option<&'static str>,
        telemetry: &RequestTelemetry,
        start: Instant,
    ) -> Result<Response<Incoming>, SequencerError> {
        let uri = self.build_uri().map_err(|error| {
            record_failed_request(telemetry, None, error_kind::REQUEST_BUILD, start.elapsed(), &error);
            error
        })?;

        let mut req_builder = Request::builder().method(http_method).uri(uri);

        req_builder.headers_mut().expect("Failed to get mutable reference to request headers").extend(self.headers);

        if let Some(content_type) = content_type {
            req_builder = req_builder.header(CONTENT_TYPE, content_type);
        }

        let req = req_builder.body(Full::new(body)).map_err(|http_error| {
            let error = SequencerError::HttpError(http_error);
            record_failed_request(telemetry, None, error_kind::REQUEST_BUILD, start.elapsed(), &error);
            error
        })?;

        self.client.clone().call(req).await.map_err(|error| {
            let error = SequencerError::HttpCallError(error);
            record_failed_request(telemetry, None, error_kind::TRANSPORT, start.elapsed(), &error);
            error
        })
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

#[derive(Clone, Copy, Debug)]
struct RequestTelemetry {
    labels: RequestLabels,
    http_method: &'static str,
}

async fn unpack<T>(
    response: Response<Incoming>,
    telemetry: &RequestTelemetry,
    start: Instant,
) -> Result<T, SequencerError>
where
    T: ::serde::de::DeserializeOwned,
{
    let http_status = response.status();
    let whole_body = response
        .collect()
        .await
        .map_err(|error| {
            let error = SequencerError::HyperError(error);
            record_failed_request(telemetry, Some(http_status), error_kind::TRANSPORT, start.elapsed(), &error);
            error
        })?
        .aggregate();

    if http_status == StatusCode::TOO_MANY_REQUESTS {
        let error = StarknetError::rate_limited();
        record_starknet_failure(telemetry, http_status, start.elapsed(), &error);
        return Err(SequencerError::StarknetError(error));
    } else if !http_status.is_success() {
        let starknet_error =
            serde_json::from_reader::<_, StarknetError>(whole_body.reader()).map_err(|serde_error| {
                let error = SequencerError::InvalidStarknetError { http_status, serde_error };
                record_failed_request(
                    telemetry,
                    Some(http_status),
                    error_kind::INVALID_STARKNET,
                    start.elapsed(),
                    &error,
                );
                error
            })?;

        record_starknet_failure(telemetry, http_status, start.elapsed(), &starknet_error);
        return Err(starknet_error.into());
    }

    let res = serde_json::from_reader(whole_body.reader()).map_err(|serde_error| {
        let error = SequencerError::DeserializeBody { serde_error };
        record_failed_request(telemetry, Some(http_status), error_kind::RESPONSE_DESERIALIZE, start.elapsed(), &error);
        error
    })?;

    record_successful_request(telemetry, http_status, start.elapsed());

    Ok(res)
}

fn record_successful_request(telemetry: &RequestTelemetry, http_status: StatusCode, duration: Duration) {
    metrics().record_request(
        telemetry.labels,
        telemetry.http_method,
        request_result::SUCCESS,
        error_kind::NONE,
        Some(http_status),
        duration,
    );
}

fn record_starknet_failure(
    telemetry: &RequestTelemetry,
    http_status: StatusCode,
    duration: Duration,
    error: &StarknetError,
) {
    metrics().record_request(
        telemetry.labels,
        telemetry.http_method,
        request_result::FAILURE,
        error_kind::STARKNET,
        Some(http_status),
        duration,
    );

    tracing::warn!(
        target: "gateway_client_errors",
        service = telemetry.labels.service,
        endpoint = telemetry.labels.endpoint,
        http_method = telemetry.http_method,
        status_code = http_status.as_u16(),
        error_kind = error_kind::STARKNET,
        starknet_error_code = ?error.code,
        duration_ms = duration.as_secs_f64() * 1000.0,
        message = %error.message,
        "Gateway client request failed"
    );
}

fn record_failed_request(
    telemetry: &RequestTelemetry,
    http_status: Option<StatusCode>,
    failure_kind: &'static str,
    duration: Duration,
    error: &impl std::fmt::Display,
) {
    metrics().record_request(
        telemetry.labels,
        telemetry.http_method,
        request_result::FAILURE,
        failure_kind,
        http_status,
        duration,
    );

    let status_code = http_status.map(|status| status.as_u16()).unwrap_or_default();
    let log_target = "gateway_client_errors";
    let duration_ms = duration.as_secs_f64() * 1000.0;

    if matches!(failure_kind, error_kind::INVALID_STARKNET) {
        tracing::warn!(
            target: log_target,
            service = telemetry.labels.service,
            endpoint = telemetry.labels.endpoint,
            http_method = telemetry.http_method,
            status_code = status_code,
            error_kind = failure_kind,
            duration_ms = duration_ms,
            error = %error,
            "Gateway client request failed"
        );
    } else {
        tracing::error!(
            target: log_target,
            service = telemetry.labels.service,
            endpoint = telemetry.labels.endpoint,
            http_method = telemetry.http_method,
            status_code = status_code,
            error_kind = failure_kind,
            duration_ms = duration_ms,
            error = %error,
            "Gateway client request failed"
        );
    }
}
