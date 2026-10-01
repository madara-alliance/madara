use super::builder::PausedClient;
use bincode::Options;
use bytes::{Buf, Bytes};
use flate2::read::MultiGzDecoder;
use http::Method;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::{Body, Incoming};
use hyper::header::{HeaderName, HeaderValue, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE};
use hyper::{HeaderMap, Request, Response, StatusCode, Uri};
use mp_gateway::error::{SequencerError, StarknetError};
use mp_rpc::v0_8_1::{BlockId, BlockTag};
use serde::de::DeserializeOwned;
use serde::Serialize;
use starknet_types_core::felt::Felt;
use std::{borrow::Cow, collections::HashMap, io::Read};
use tower::Service;
use url::Url;

const MAX_FEEDER_RESPONSE_BODY_BYTES: usize = 64 * 1024 * 1024;

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
        unpack(self.send_get_raw().await?).await
    }

    pub async fn send_get_raw(self) -> Result<Response<Incoming>, SequencerError> {
        let mut client = self.client.clone();
        let req = self.build_get_request()?;

        let response: Response<Incoming> = client.call(req).await.map_err(SequencerError::HttpCallError)?;
        Ok(response)
    }

    fn build_get_request(mut self) -> Result<Request<Full<Bytes>>, SequencerError> {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method(Method::GET).uri(uri);

        self.headers.entry(ACCEPT_ENCODING).or_insert(HeaderValue::from_static("gzip"));
        req_builder.headers_mut().expect("Failed to get mutable reference to request headers").extend(self.headers);

        let req = req_builder.body(Full::new(Bytes::from(String::new())))?;
        Ok(req)
    }

    pub async fn send_post_bincode<T, D>(self, body: D) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
        D: Serialize,
    {
        let uri = self.build_uri()?;

        let mut req_builder = Request::builder().method(Method::POST).uri(uri);

        req_builder.headers_mut().expect("Failed to get mutable reference to request headers").extend(self.headers);

        let body = bincode::options()
            .with_little_endian()
            .serialize(&body)
            .map_err(|err| SequencerError::HttpCallError(err))?; // Fixed endinaness is important.
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

        let res = bincode::options()
            .with_little_endian() // Fixed endinaness is important.
            .deserialize_from(whole_body.reader())
            .map_err(|err| SequencerError::HttpCallError(err))?;

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
    let headers = response.headers().clone();
    let whole_body = collect_response_body(response.into_body(), MAX_FEEDER_RESPONSE_BODY_BYTES).await?;
    unpack_bytes(http_status, &headers, whole_body)
}

async fn collect_response_body<B>(body: B, max_bytes: usize) -> Result<Bytes, SequencerError>
where
    B: Body<Data = Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    Limited::new(body, max_bytes).collect().await.map(|body| body.to_bytes()).map_err(|source| {
        if source.downcast_ref::<LengthLimitError>().is_some() {
            SequencerError::ResponseBodyTooLarge { max_bytes }
        } else {
            SequencerError::HttpCallError(source)
        }
    })
}

fn unpack_bytes<T>(http_status: StatusCode, headers: &HeaderMap, body: Bytes) -> Result<T, SequencerError>
where
    T: ::serde::de::DeserializeOwned,
{
    let body = decode_response_body(headers, body)?;

    if http_status == StatusCode::TOO_MANY_REQUESTS {
        return Err(SequencerError::StarknetError(StarknetError::rate_limited()));
    } else if !http_status.is_success() {
        let starknet_error = serde_json::from_slice::<StarknetError>(&body)
            .map_err(|serde_error| SequencerError::InvalidStarknetError { http_status, serde_error })?;

        return Err(starknet_error.into());
    }

    serde_json::from_slice(&body).map_err(|serde_error| SequencerError::DeserializeBody { serde_error })
}

fn decode_response_body(headers: &HeaderMap, body: Bytes) -> Result<Bytes, SequencerError> {
    decode_response_body_with_limit(headers, body, MAX_FEEDER_RESPONSE_BODY_BYTES)
}

fn decode_response_body_with_limit(
    headers: &HeaderMap,
    body: Bytes,
    max_bytes: usize,
) -> Result<Bytes, SequencerError> {
    let gzip_encoded = headers
        .get_all(CONTENT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|encoding| encoding.trim().eq_ignore_ascii_case("gzip"));

    if !gzip_encoded {
        return Ok(body);
    }

    let mut decoder = MultiGzDecoder::new(body.as_ref()).take(max_bytes.saturating_add(1) as u64);
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded).map_err(|source| SequencerError::DecompressResponse { source })?;
    if decoded.len() > max_bytes {
        return Err(SequencerError::ResponseBodyTooLarge { max_bytes });
    }
    Ok(decoded.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use mp_gateway::error::StarknetErrorCode;
    use serde_json::json;
    use std::{io::Write, net::TcpListener, thread};

    fn gzip(body: &[u8]) -> Bytes {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(body).expect("write gzip body");
        encoder.finish().expect("finish gzip body").into()
    }

    #[test]
    fn feeder_get_advertises_gzip_by_default() {
        let gateway_url = Url::parse("http://127.0.0.1:1/gateway/").unwrap();
        let feeder_url = Url::parse("http://127.0.0.1:1/feeder_gateway/").unwrap();
        let provider = super::super::builder::GatewayProvider::new(gateway_url, feeder_url.clone());

        let request = RequestBuilder::new(&provider.client, feeder_url, provider.headers.clone())
            .add_uri_segment("get_block")
            .unwrap()
            .build_get_request()
            .unwrap();

        assert_eq!(request.headers().get(ACCEPT_ENCODING), Some(&HeaderValue::from_static("gzip")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_feeder_round_trips_gzip_json() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected = json!({"block_number": 42});
        let compressed = gzip(&serde_json::to_vec(&expected).unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 512];
                let read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..read]);
                if read == 0 || request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.lines().any(|line| line.eq_ignore_ascii_case("accept-encoding: gzip")));

            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                compressed.len()
            );
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(&compressed).unwrap();
        });

        let provider = super::super::builder::GatewayProvider::new_from_base_path(
            Url::parse(&format!("http://{address}/")).unwrap(),
        );
        let response: serde_json::Value =
            RequestBuilder::new(&provider.client, provider.feeder_gateway_url.clone(), provider.headers.clone())
                .add_uri_segment("get_block")
                .unwrap()
                .send_get()
                .await
                .unwrap();

        server.join().unwrap();
        assert_eq!(response, expected);
    }

    #[test]
    fn response_decoding_supports_plain_and_gzip_json() {
        let expected = json!({"block_number": 42});
        let plain = serde_json::to_vec(&expected).unwrap();
        let plain_result: serde_json::Value =
            unpack_bytes(StatusCode::OK, &HeaderMap::new(), plain.clone().into()).unwrap();
        let mut identity_headers = HeaderMap::new();
        identity_headers.insert(CONTENT_ENCODING, HeaderValue::from_static("identity"));
        let identity_result: serde_json::Value =
            unpack_bytes(StatusCode::OK, &identity_headers, plain.clone().into()).unwrap();

        let mut gzip_headers = HeaderMap::new();
        gzip_headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let gzip_result: serde_json::Value = unpack_bytes(StatusCode::OK, &gzip_headers, gzip(&plain)).unwrap();

        assert_eq!(plain_result, expected);
        assert_eq!(identity_result, expected);
        assert_eq!(gzip_result, expected);
    }

    #[test]
    fn response_decoding_supports_concatenated_gzip_members() {
        let expected = json!({"block_number": 42});
        let mut body = Vec::new();
        body.extend_from_slice(&gzip(br#"{"block_number":"#));
        body.extend_from_slice(&gzip(b"42}"));
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));

        let result: serde_json::Value = unpack_bytes(StatusCode::OK, &headers, body.into()).unwrap();

        assert_eq!(result, expected);
    }

    #[test]
    fn gzip_structured_errors_are_decoded_before_mapping() {
        let error = StarknetError::block_not_found();
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));

        let result = unpack_bytes::<serde_json::Value>(
            StatusCode::BAD_REQUEST,
            &headers,
            gzip(&serde_json::to_vec(&error).unwrap()),
        );

        assert!(matches!(
            result,
            Err(SequencerError::StarknetError(StarknetError { code: StarknetErrorCode::BlockNotFound, .. }))
        ));
    }

    #[test]
    fn invalid_and_truncated_gzip_return_typed_errors() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let valid = gzip(br#"{"ok":true}"#);
        let truncated = valid.slice(..valid.len() / 2);

        for body in [Bytes::from_static(b"not gzip"), truncated] {
            let result = unpack_bytes::<serde_json::Value>(StatusCode::OK, &headers, body);
            assert!(matches!(result, Err(SequencerError::DecompressResponse { .. })));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn compressed_and_decompressed_response_sizes_are_bounded() {
        let compressed = gzip(&[b'a'; 64]);
        let raw_result = collect_response_body(Full::new(compressed.clone()), compressed.len() - 1).await;
        assert!(matches!(
            raw_result,
            Err(SequencerError::ResponseBodyTooLarge { max_bytes }) if max_bytes == compressed.len() - 1
        ));

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let decoded_result = decode_response_body_with_limit(&headers, compressed, 32);
        assert!(matches!(decoded_result, Err(SequencerError::ResponseBodyTooLarge { max_bytes: 32 })));
    }
}
