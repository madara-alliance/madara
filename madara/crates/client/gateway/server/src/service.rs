use super::{metrics::GatewayMetrics, router::main_router};
use anyhow::Context;
use bytes::Bytes;
use flate2::{write::GzEncoder, Compression};
use http_body_util::Full;
use hyper::{
    header::{HeaderMap, HeaderValue, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, VARY},
    server::conn::http1,
    service::service_fn,
    Response,
};
use hyper_util::rt::TokioIo;
use mc_db::MadaraBackend;
use mc_submit_tx::{SubmitTransaction, SubmitValidatedTransaction, TransactionLookup};
use mp_utils::service::ServiceContext;
use std::{
    convert::Infallible,
    io::{self, Write},
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Instant,
};
use tokio::{net::TcpListener, sync::Semaphore};

const MAX_CONCURRENT_GZIP_COMPRESSIONS: usize = 4;

#[derive(Debug, Clone)]
pub struct GatewayServerConfig {
    pub feeder_gateway_enable: bool,
    pub feeder_gateway_gzip_responses: bool,
    pub gateway_enable: bool,
    pub gateway_external: bool,
    pub gateway_port: u16,
    pub enable_trusted_add_validated_transaction: bool,
}
impl Default for GatewayServerConfig {
    fn default() -> Self {
        Self {
            feeder_gateway_enable: false,
            feeder_gateway_gzip_responses: false,
            gateway_enable: false,
            gateway_external: false,
            gateway_port: 8080,
            enable_trusted_add_validated_transaction: false,
        }
    }
}

pub async fn start_server(
    db_backend: Arc<MadaraBackend>,
    transaction_submitter: Arc<dyn SubmitTransaction>,
    transaction_lookup: Arc<dyn TransactionLookup>,
    submit_validated: Option<Arc<dyn SubmitValidatedTransaction>>,
    mut ctx: ServiceContext,
    config: GatewayServerConfig,
) -> anyhow::Result<()> {
    if !config.feeder_gateway_enable && !config.gateway_enable && !config.enable_trusted_add_validated_transaction {
        return Ok(());
    }

    let listen_addr = if config.gateway_external {
        Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
    } else {
        Ipv4Addr::LOCALHOST
    };
    let addr = SocketAddr::new(listen_addr.into(), config.gateway_port);
    let listener = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;

    let addr = listener.local_addr().context("Getting the bound-to address.")?;
    tracing::info!("🌐 Gateway endpoint started at {}", addr);
    let gzip_compression_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_GZIP_COMPRESSIONS));
    let gateway_metrics = GatewayMetrics::register();

    while let Some(res) = ctx.run_until_cancelled(listener.accept()).await {
        // Handle new incoming connections
        if let Ok((stream, _)) = res {
            let io = TokioIo::new(stream);

            let db_backend = Arc::clone(&db_backend);
            let transaction_submitter = transaction_submitter.clone();
            let transaction_lookup = transaction_lookup.clone();
            let submit_validated = submit_validated.clone();
            let config = config.clone();
            let gzip_compression_semaphore = Arc::clone(&gzip_compression_semaphore);
            let gateway_metrics = gateway_metrics.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let db_backend = Arc::clone(&db_backend);
                    let transaction_submitter = transaction_submitter.clone();
                    let transaction_lookup = transaction_lookup.clone();
                    let submit_validated = submit_validated.clone();
                    let config = config.clone();
                    let gzip_compression_semaphore = Arc::clone(&gzip_compression_semaphore);
                    let gateway_metrics = gateway_metrics.clone();
                    async move {
                        let path = req
                            .uri()
                            .path()
                            .split('/')
                            .filter(|segment| !segment.is_empty())
                            .collect::<Vec<_>>()
                            .join("/");
                        let request_headers = req.headers().clone();
                        let gzip_enabled = config.feeder_gateway_gzip_responses;
                        let telemetry_route = telemetry_route(&path);
                        let start = Instant::now();
                        let Ok(res) = main_router(
                            req,
                            &path,
                            db_backend,
                            transaction_submitter,
                            transaction_lookup,
                            submit_validated,
                            config,
                        )
                        .await;

                        let (res, response_stats) =
                            prepare_response(&request_headers, &path, res, gzip_enabled, gzip_compression_semaphore)
                                .await;
                        let status = res.status().as_u16() as i64;
                        let response_time = start.elapsed().as_micros();

                        if path.starts_with("feeder_gateway/") {
                            gateway_metrics.record_response(
                                telemetry_route,
                                response_stats.encoding,
                                response_stats.uncompressed_bytes,
                                response_stats.transmitted_bytes,
                            );
                        }

                        tracing::info!(
                            target: "gateway_calls",
                            method = telemetry_route,
                            status = status,
                            encoding = response_stats.encoding,
                            uncompressed_bytes = response_stats.uncompressed_bytes,
                            transmitted_bytes = response_stats.transmitted_bytes,
                            compression_duration = response_stats.compression_duration,
                            res_len = response_stats.transmitted_bytes,
                            response_time = response_time,
                            "{telemetry_route} {status} {} bytes logical / {} bytes transmitted ({}, compression {} micros) - {response_time} micros",
                            response_stats.uncompressed_bytes,
                            response_stats.transmitted_bytes,
                            response_stats.encoding,
                            response_stats.compression_duration,
                        );

                        Ok::<_, Infallible>(res)
                    }
                });

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("Error serving connection: {:#}", err);
                }
            });
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ResponseStats {
    encoding: &'static str,
    uncompressed_bytes: u64,
    transmitted_bytes: u64,
    compression_duration: u128,
}

fn telemetry_route(path: &str) -> &'static str {
    match path {
        "health" => "health",
        "gateway/add_transaction" => "gateway/add_transaction",
        "madara/trusted_add_validated_transaction" => "madara/trusted_add_validated_transaction",
        "feeder_gateway/get_preconfirmed_block" => "feeder_gateway/get_preconfirmed_block",
        "feeder_gateway/get_block" => "feeder_gateway/get_block",
        "feeder_gateway/get_signature" => "feeder_gateway/get_signature",
        "feeder_gateway/get_state_update" => "feeder_gateway/get_state_update",
        "feeder_gateway/get_transaction" => "feeder_gateway/get_transaction",
        "feeder_gateway/get_transaction_status" => "feeder_gateway/get_transaction_status",
        "feeder_gateway/get_block_hash_by_id" => "feeder_gateway/get_block_hash_by_id",
        "feeder_gateway/get_block_id_by_hash" => "feeder_gateway/get_block_id_by_hash",
        "feeder_gateway/get_block_traces" => "feeder_gateway/get_block_traces",
        "feeder_gateway/get_class_by_hash" => "feeder_gateway/get_class_by_hash",
        "feeder_gateway/get_compiled_class_by_class_hash" => "feeder_gateway/get_compiled_class_by_class_hash",
        "feeder_gateway/get_contract_addresses" => "feeder_gateway/get_contract_addresses",
        "feeder_gateway/get_public_key" => "feeder_gateway/get_public_key",
        "feeder_gateway/get_block_bouncer_weights" => "feeder_gateway/get_block_bouncer_weights",
        _ => "unknown",
    }
}

async fn prepare_response(
    request_headers: &HeaderMap,
    path: &str,
    response: Response<String>,
    gzip_enabled: bool,
    gzip_compression_semaphore: Arc<Semaphore>,
) -> (Response<Full<Bytes>>, ResponseStats) {
    let uncompressed_bytes = response.body().len() as u64;
    let should_compress = gzip_enabled
        && path.starts_with("feeder_gateway/")
        && response.status().is_success()
        && uncompressed_bytes > 0
        && accepts_gzip(request_headers);

    if !should_compress {
        return identity_response(response, 0);
    }

    let Ok(permit) = gzip_compression_semaphore.try_acquire_owned() else {
        tracing::warn!(target: "gateway_calls", method = telemetry_route(path), "Feeder gzip concurrency limit reached; returning identity");
        return identity_response(response, 0);
    };

    let (parts, body) = response.into_parts();
    let body = Arc::new(body);
    let body_to_compress = Arc::clone(&body);
    let compression_start = Instant::now();
    let compressed = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        gzip(body_to_compress.as_bytes())
    })
    .await
    .map_err(io::Error::other)
    .and_then(|result| result);
    let compression_duration = compression_start.elapsed().as_micros();

    finish_compression(parts, body, compressed, compression_duration)
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    let mut gzip_quality = None;
    let mut wildcard_quality = None;

    for value in headers.get_all(ACCEPT_ENCODING) {
        let Ok(value) = value.to_str() else { continue };
        for encoding in value.split(',') {
            let mut parts = encoding.split(';');
            let token = parts.next().unwrap_or_default().trim();
            let mut quality = 1000;
            for parameter in parts {
                let Some((name, value)) = parameter.split_once('=') else { continue };
                if name.trim().eq_ignore_ascii_case("q") {
                    quality = parse_quality(value.trim()).unwrap_or(0);
                }
            }

            if token.eq_ignore_ascii_case("gzip") {
                gzip_quality = Some(gzip_quality.unwrap_or(0).max(quality));
            } else if token == "*" {
                wildcard_quality = Some(wildcard_quality.unwrap_or(0).max(quality));
            }
        }
    }

    gzip_quality.or(wildcard_quality).unwrap_or(0) > 0
}

fn parse_quality(value: &str) -> Option<u16> {
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if fractional.len() > 3 || !fractional.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    match whole {
        "0" => {
            let mut padded = fractional.to_owned();
            padded.push_str(&"0".repeat(3 - padded.len()));
            padded.parse().ok()
        }
        "1" if fractional.bytes().all(|byte| byte == b'0') => Some(1000),
        _ => None,
    }
}

fn gzip(body: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(body)?;
    encoder.finish()
}

fn identity_response(response: Response<String>, compression_duration: u128) -> (Response<Full<Bytes>>, ResponseStats) {
    let (parts, body) = response.into_parts();
    let bytes = Bytes::from(body);
    let transmitted_bytes = bytes.len() as u64;
    (
        Response::from_parts(parts, Full::new(bytes)),
        ResponseStats {
            encoding: "identity",
            uncompressed_bytes: transmitted_bytes,
            transmitted_bytes,
            compression_duration,
        },
    )
}

fn finish_compression(
    mut parts: hyper::http::response::Parts,
    body: Arc<String>,
    compressed: io::Result<Vec<u8>>,
    compression_duration: u128,
) -> (Response<Full<Bytes>>, ResponseStats) {
    let uncompressed_bytes = body.len() as u64;
    match compressed {
        Ok(compressed) => {
            let transmitted_bytes = compressed.len() as u64;
            parts.headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            append_vary_accept_encoding(&mut parts.headers);
            parts.headers.insert(CONTENT_LENGTH, HeaderValue::from(transmitted_bytes));
            (
                Response::from_parts(parts, Full::new(compressed.into())),
                ResponseStats { encoding: "gzip", uncompressed_bytes, transmitted_bytes, compression_duration },
            )
        }
        Err(error) => {
            tracing::error!(target: "gateway_errors", %error, "Failed to compress feeder response; returning identity");
            let body = match Arc::try_unwrap(body) {
                Ok(body) => Bytes::from(body),
                Err(body) => Bytes::copy_from_slice(body.as_bytes()),
            };
            (
                Response::from_parts(parts, Full::new(body)),
                ResponseStats {
                    encoding: "identity",
                    uncompressed_bytes,
                    transmitted_bytes: uncompressed_bytes,
                    compression_duration,
                },
            )
        }
    }
}

fn append_vary_accept_encoding(headers: &mut HeaderMap) {
    let already_varies = headers
        .get_all(VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|name| name.trim() == "*" || name.trim().eq_ignore_ascii_case("accept-encoding"));
    if !already_varies {
        headers.append(VARY, HeaderValue::from_static("Accept-Encoding"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use http_body_util::BodyExt;
    use hyper::{header::CONTENT_TYPE, StatusCode};
    use std::io::Read;

    fn headers(values: &[&'static str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(ACCEPT_ENCODING, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn parses_accept_encoding_with_http_quality_semantics() {
        assert!(!accepts_gzip(&HeaderMap::new()));
        assert!(accepts_gzip(&headers(&["GZip"])));
        assert!(!accepts_gzip(&headers(&["br, gzip;q=0"])));
        assert!(accepts_gzip(&headers(&["br", "deflate, gzip; q=0.25"])));
        assert!(accepts_gzip(&headers(&["br, *;q=0.5"])));
        assert!(!accepts_gzip(&headers(&["gzip;q=0", "*;q=1"])));
        assert!(!accepts_gzip(&headers(&["gzip;q=1.1"])));
    }

    #[test]
    fn telemetry_routes_are_bounded() {
        assert_eq!(telemetry_route("feeder_gateway/get_block"), "feeder_gateway/get_block");
        assert_eq!(telemetry_route("feeder_gateway/unbounded-user-input"), "unknown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn compresses_eligible_feeder_responses_and_preserves_metadata() {
        let original = serde_json::json!({"blocks": vec!["repetitive response"; 128]}).to_string();
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .header("x-test", "preserved")
            .header(VARY, "Origin")
            .body(original.clone())
            .unwrap();

        let (response, stats) = prepare_response(
            &headers(&["br, gzip"]),
            "feeder_gateway/get_block",
            response,
            true,
            Arc::new(Semaphore::new(1)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(response.headers().get("x-test").unwrap(), "preserved");
        assert_eq!(response.headers().get(CONTENT_ENCODING).unwrap(), "gzip");
        assert_eq!(response.headers().get(CONTENT_LENGTH).unwrap(), stats.transmitted_bytes.to_string().as_str());
        let vary = response.headers().get_all(VARY).iter().map(|value| value.to_str().unwrap()).collect::<Vec<_>>();
        assert_eq!(vary, ["Origin", "Accept-Encoding"]);
        assert_eq!(stats.encoding, "gzip");
        assert_eq!(stats.uncompressed_bytes, original.len() as u64);
        assert!(stats.transmitted_bytes < stats.uncompressed_bytes);

        let compressed = response.into_body().collect().await.unwrap().to_bytes();
        let mut decoded = String::new();
        GzDecoder::new(compressed.as_ref()).read_to_string(&mut decoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn leaves_ineligible_responses_uncompressed() {
        let cases = [
            (HeaderMap::new(), "feeder_gateway/get_block", StatusCode::OK, true, "original"),
            (headers(&["gzip"]), "feeder_gateway/get_block", StatusCode::OK, false, "original"),
            (headers(&["gzip"]), "health", StatusCode::OK, true, "original"),
            (headers(&["gzip"]), "gateway/add_transaction", StatusCode::OK, true, "original"),
            (headers(&["gzip"]), "feeder_gateway/get_block", StatusCode::BAD_REQUEST, true, "original"),
            (headers(&["gzip"]), "feeder_gateway/get_block", StatusCode::OK, true, ""),
        ];

        for (headers, path, status, enabled, body) in cases {
            let response = Response::builder().status(status).body(body.to_string()).unwrap();
            let (response, stats) =
                prepare_response(&headers, path, response, enabled, Arc::new(Semaphore::new(1))).await;
            assert_eq!(response.headers().get(CONTENT_ENCODING), None);
            assert_eq!(stats.encoding, "identity");
            assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), body);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrency_saturation_falls_back_to_identity() {
        let response = Response::new("original".to_string());
        let (response, stats) = prepare_response(
            &headers(&["gzip"]),
            "feeder_gateway/get_block",
            response,
            true,
            Arc::new(Semaphore::new(0)),
        )
        .await;

        assert_eq!(stats.encoding, "identity");
        assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "original");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn compression_failure_falls_back_without_corrupting_response() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body("original".to_string())
            .unwrap();
        let (parts, body) = response.into_parts();
        let (response, stats) = finish_compression(parts, Arc::new(body), Err(io::Error::other("test failure")), 7);

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(response.headers().get(CONTENT_ENCODING), None);
        assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "original");
        assert_eq!(
            stats,
            ResponseStats {
                encoding: "identity",
                uncompressed_bytes: 8,
                transmitted_bytes: 8,
                compression_duration: 7,
            }
        );
    }
}
