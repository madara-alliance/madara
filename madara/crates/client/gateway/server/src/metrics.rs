use mc_analytics::register_gauge_metric_instrument;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

/// Metrics for Gateway server tracking concurrent requests.
#[derive(Debug, Clone)]
pub struct GatewayMetrics {
    /// Number of concurrent Gateway requests.
    pub concurrent_requests: Gauge<u64>,
    /// Internal counter for tracking concurrent requests.
    concurrent_requests_count: Arc<AtomicU64>,
}

impl GatewayMetrics {
    /// Create an instance of metrics
    pub fn register() -> anyhow::Result<Self> {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.gateway.opentelemetry")
                .with_attributes([KeyValue::new("crate", "gateway")])
                .build(),
        );

        let concurrent_requests = register_gauge_metric_instrument(
            &meter,
            "gateway_concurrent_requests".to_string(),
            "Number of concurrent Gateway requests being processed".to_string(),
            "request".to_string(),
        );

        Ok(Self { concurrent_requests, concurrent_requests_count: Arc::new(AtomicU64::new(0)) })
    }

    pub(crate) fn on_request_start(&self) {
        // Update concurrent requests metric
        let current = self.concurrent_requests_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.concurrent_requests.record(current, &[]);
    }

    pub(crate) fn on_request_end(&self) {
        // Update concurrent requests metric
        let current = self.concurrent_requests_count.fetch_sub(1, Ordering::Relaxed) - 1;
        self.concurrent_requests.record(current, &[]);
    }
}
