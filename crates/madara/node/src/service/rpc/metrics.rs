use std::time::Instant;

use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use opentelemetry::{
    global::Error,
    metrics::{Counter, Histogram},
};

use mc_analytics::{register_counter_metric_instrument, register_histogram_metric_instrument};
use opentelemetry::{global, KeyValue};

/// Metrics for RPC middleware storing information about the number of requests started/completed,
/// calls started/completed and their timings.
#[derive(Debug, Clone)]
pub struct RpcMetrics {
    /// Histogram over RPC execution times.
    calls_time: Histogram<f64>,
    /// Number of calls started.
    calls_started: Counter<u64>,
    /// Number of calls completed.
    calls_finished: Counter<u64>,
    /// Number of Websocket sessions opened.
    ws_sessions_opened: Option<Counter<u64>>,
    /// Number of Websocket sessions closed.
    ws_sessions_closed: Option<Counter<u64>>,
    /// Histogram over RPC websocket sessions.
    ws_sessions_time: Histogram<f64>,
}

impl RpcMetrics {
    /// Create an instance of metrics
    pub fn register() -> Result<Self, Error> {
        let common_scope_attributes = vec![KeyValue::new("crate", "rpc")];
        let rpc_meter = global::meter_with_version(
            "crates.rpc.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let calls_started = register_counter_metric_instrument(
            &rpc_meter,
            "calls_started".to_string(),
            "A counter to show block state at given time".to_string(),
            "".to_string(),
        );

        let calls_finished = register_counter_metric_instrument(
            &rpc_meter,
            "calls_finished".to_string(),
            "A counter to show block state at given time".to_string(),
            "".to_string(),
        );

        let calls_time = register_histogram_metric_instrument(
            &rpc_meter,
            "calls_time".to_string(),
            "A histogram to show the time taken for RPC calls".to_string(),
            "".to_string(),
        );

        let ws_sessions_opened = Some(register_counter_metric_instrument(
            &rpc_meter,
            "ws_sessions_opened".to_string(),
            "A counter to show the number of websocket sessions opened".to_string(),
            "".to_string(),
        ));

        let ws_sessions_closed = Some(register_counter_metric_instrument(
            &rpc_meter,
            "ws_sessions_closed".to_string(),
            "A counter to show the number of websocket sessions closed".to_string(),
            "".to_string(),
        ));

        let ws_sessions_time = register_histogram_metric_instrument(
            &rpc_meter,
            "ws_sessions_time".to_string(),
            "A histogram to show the time taken for RPC websocket sessions".to_string(),
            "".to_string(),
        );

        Ok(Self { calls_time, calls_started, calls_finished, ws_sessions_opened, ws_sessions_closed, ws_sessions_time })
    }

    pub(crate) fn ws_connect(&self) {
        if let Some(counter) = self.ws_sessions_opened.as_ref() {
            counter.add(1, &[]);
        }
    }

    pub(crate) fn ws_disconnect(&self, now: Instant) {
        let millis = now.elapsed().as_millis();

        if let Some(counter) = self.ws_sessions_closed.as_ref() {
            counter.add(1, &[]);
        }
        self.ws_sessions_time.record(millis as f64, &[]);
    }

    pub(crate) fn on_call(&self, req: &Request, transport_label: &'static str) {
        tracing::trace!(
            target: "rpc_metrics",
            "[{transport_label}] on_call name={} params={:?}",
            req.method_name(),
            req.params(),
        );
        self.calls_started.add(1, &[KeyValue::new("method", req.method_name().to_string())]);
    }

    pub(crate) fn on_response(&self, req: &Request, rp: &MethodResponse, transport_label: &'static str, now: Instant) {
        tracing::trace!(target: "rpc_metrics", "[{transport_label}] on_response started_at={:?}", now);
        tracing::trace!(target: "rpc_metrics::extra", "[{transport_label}] result={}", rp.as_result());

        let millis = now.elapsed().as_millis();
        tracing::debug!(
            target: "rpc_metrics",
            "[{transport_label}] {} call took {:?}",
            req.method_name(),
            millis,
        );

        self.calls_time.record(millis as f64, &[KeyValue::new("method", req.method_name().to_string())]);

        self.calls_finished.add(
            1,
            &[
                KeyValue::new("method", req.method_name().to_string()),
                KeyValue::new("success", rp.is_success().to_string()),
            ],
        );
    }
}

/// Metrics with transport label.
#[derive(Clone, Debug)]
pub struct Metrics {
    pub(crate) inner: RpcMetrics,
    pub(crate) transport_label: &'static str,
}

impl Metrics {
    /// Create a new [`Metrics`].
    pub fn new(metrics: RpcMetrics, transport_label: &'static str) -> Self {
        Self { inner: metrics, transport_label }
    }

    pub(crate) fn ws_connect(&self) {
        self.inner.ws_connect();
    }

    pub(crate) fn ws_disconnect(&self, now: Instant) {
        self.inner.ws_disconnect(now)
    }

    pub(crate) fn on_call(&self, req: &Request) {
        self.inner.on_call(req, self.transport_label)
    }

    pub(crate) fn on_response(&self, req: &Request, rp: &MethodResponse, now: Instant) {
        self.inner.on_response(req, rp, self.transport_label, now)
    }
}
