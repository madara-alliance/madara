use axum::{body::Body, http::HeaderValue, http::Request, middleware::Next, response::Response};
use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt, Context as OtelContext};
use rand::RngCore;
use tracing::info_span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

/// Middleware that ensures every incoming request has a trace span and a trace ID.
///
/// - Creates a new tracing span per request with common fields
/// - Derives an OpenTelemetry trace_id when available, otherwise generates a UUID
/// - Propagates `x-trace-id` header to downstream handlers and the response
/// - Declares an empty `job_id` field on the span so handlers can populate it
pub async fn trace_context(mut req: Request<Body>, next: Next) -> Response {
    // Determine preferred trace id: incoming header if valid, else generate one
    let incoming = req.headers().get("x-trace-id").and_then(|h| h.to_str().ok()).map(|s| s.trim().to_lowercase());

    let chosen_trace_id =
        incoming.filter(|s| s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())).unwrap_or_else(|| {
            // Fallback: generate from UUIDv4 to 32-hex without dashes
            Uuid::new_v4().simple().to_string()
        });

    // Build a parent OpenTelemetry context from the chosen trace id using W3C traceparent format
    // traceparent: 00-<trace-id 32hex>-<span-id 16hex>-01
    let mut span_id_bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut span_id_bytes);
    let span_id_hex = hex::encode(span_id_bytes);
    let traceparent_val = format!("00-{}-{}-01", chosen_trace_id, span_id_hex);

    // Minimal carrier for propagation extraction
    struct SimpleCarrier<'a> {
        tp: &'a str,
    }
    impl Extractor for SimpleCarrier<'_> {
        fn get(&self, key: &str) -> Option<&str> {
            if key.eq_ignore_ascii_case("traceparent") {
                Some(self.tp)
            } else {
                None
            }
        }
        fn keys(&self) -> Vec<&str> {
            vec!["traceparent"]
        }
    }
    let parent_cx: OtelContext =
        global::get_text_map_propagator(|prop| prop.extract(&SimpleCarrier { tp: &traceparent_val }));

    // Create the server span and assign the extracted/generated parent so OTEL uses our trace id
    let span = info_span!(
        "http_request",
        method = %req.method(),
        path = %req.uri().path(),
        trace_id = tracing::field::Empty,
        otel_trace_id = tracing::field::Empty,
        job_id = tracing::field::Empty
    );
    span.set_parent(parent_cx);

    // Record ids for log visibility
    let otel_trace_id_str = span.context().span().span_context().trace_id().to_string();
    span.record("trace_id", tracing::field::display(&chosen_trace_id));
    span.record("otel_trace_id", tracing::field::display(&otel_trace_id_str));

    // Ensure the request carries x-trace-id header for downstream handlers
    if let Ok(hv) = HeaderValue::from_str(&chosen_trace_id) {
        req.headers_mut().insert(axum::http::header::HeaderName::from_static("x-trace-id"), hv);
    }

    // Run downstream within span
    let mut res = span.in_scope(|| async move { next.run(req).await }).await;

    // Echo the trace id back in the response
    if let Ok(hv) = HeaderValue::from_str(&chosen_trace_id) {
        res.headers_mut().insert(axum::http::header::HeaderName::from_static("x-trace-id"), hv);
    }

    res
}
