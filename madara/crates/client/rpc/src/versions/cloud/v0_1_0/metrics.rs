/// T-064: Cloud RPC metrics.
use mc_telemetry::{register_counter_metric_instrument, register_histogram_metric_instrument};
use opentelemetry::{
    global,
    metrics::{Counter, Histogram},
    InstrumentationScope, KeyValue,
};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct CloudRpcMetrics {
    /// Total invoke transactions received.
    pub add_invoke_received_total: Counter<u64>,
    /// Total invoke transactions accepted by mempool.
    pub add_invoke_accepted_total: Counter<u64>,
    /// Total invoke transactions rejected (labelled by reason).
    pub add_invoke_rejected_total: Counter<u64>,
    /// End-to-end duration per invoke request (labelled by outcome: accepted|rejected).
    pub add_invoke_duration_seconds: Histogram<f64>,
    /// Duration of the mempool submit call (labelled by outcome: accepted|rejected).
    pub add_invoke_mempool_submit_duration_seconds: Histogram<f64>,
    /// Number of in-flight invoke requests (used as a gauge via atomic).
    pub add_invoke_inflight: Arc<AtomicI64>,
}

impl CloudRpcMetrics {
    pub fn register() -> anyhow::Result<Self> {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.rpc.cloud")
                .with_attributes([KeyValue::new("crate", "rpc_cloud")])
                .build(),
        );

        let add_invoke_received_total = register_counter_metric_instrument(
            &meter,
            "cloud_rpc_add_invoke_received_total".to_string(),
            "Total number of addInvokeTransaction requests received on cloud endpoint".to_string(),
            "".to_string(),
        );

        let add_invoke_accepted_total = register_counter_metric_instrument(
            &meter,
            "cloud_rpc_add_invoke_accepted_total".to_string(),
            "Total number of addInvokeTransaction requests accepted by mempool".to_string(),
            "".to_string(),
        );

        let add_invoke_rejected_total = register_counter_metric_instrument(
            &meter,
            "cloud_rpc_add_invoke_rejected_total".to_string(),
            "Total number of addInvokeTransaction requests rejected (labelled by reason)".to_string(),
            "".to_string(),
        );

        let add_invoke_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "cloud_rpc_add_invoke_duration_seconds".to_string(),
            "End-to-end duration of addInvokeTransaction requests in seconds".to_string(),
            "s".to_string(),
        );

        let add_invoke_mempool_submit_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "cloud_rpc_add_invoke_mempool_submit_duration_seconds".to_string(),
            "Duration of the mempool submit call for addInvokeTransaction in seconds".to_string(),
            "s".to_string(),
        );

        Ok(Self {
            add_invoke_received_total,
            add_invoke_accepted_total,
            add_invoke_rejected_total,
            add_invoke_duration_seconds,
            add_invoke_mempool_submit_duration_seconds,
            add_invoke_inflight: Arc::new(AtomicI64::new(0)),
        })
    }

    pub fn on_received(&self) {
        self.add_invoke_received_total.add(1, &[]);
        self.add_invoke_inflight.fetch_add(1, Ordering::Relaxed);
    }

    pub fn on_accepted(&self, duration_secs: f64, mempool_duration_secs: f64) {
        self.add_invoke_inflight.fetch_sub(1, Ordering::Relaxed);
        self.add_invoke_accepted_total.add(1, &[]);
        self.add_invoke_duration_seconds.record(duration_secs, &[KeyValue::new("outcome", "accepted")]);
        self.add_invoke_mempool_submit_duration_seconds
            .record(mempool_duration_secs, &[KeyValue::new("outcome", "accepted")]);
    }

    pub fn on_rejected(&self, reason: &'static str, duration_secs: f64) {
        self.add_invoke_inflight.fetch_sub(1, Ordering::Relaxed);
        self.add_invoke_rejected_total.add(1, &[KeyValue::new("reason", reason)]);
        self.add_invoke_duration_seconds.record(duration_secs, &[KeyValue::new("outcome", "rejected")]);
    }
}
