//! Metrics for transaction execution in Madara.
//!
//! This module provides execution time tracking for individual transactions.
//! Metrics are exported via OpenTelemetry (OTEL) for integration with Prometheus/OTLP.

use mc_analytics::register_histogram_metric_instrument;
use opentelemetry::metrics::Histogram;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use starknet_api::executable_transaction::TransactionType;
use std::time::Instant;

/// Transaction type labels for metrics.
pub mod tx_type_label {
    pub const INVOKE: &str = "invoke";
    pub const DECLARE: &str = "declare";
    pub const DEPLOY_ACCOUNT: &str = "deploy_account";
    pub const L1_HANDLER: &str = "l1_handler";
}

/// Execution context labels for metrics.
pub mod context_label {
    /// RPC re-execution (tracing, fee estimation, simulation).
    pub const RPC: &str = "rpc";
    /// Real block production execution.
    pub const PRODUCTION: &str = "production";
}

/// Convert TransactionType to a metric label string.
pub fn tx_type_to_label(tx_type: TransactionType) -> &'static str {
    match tx_type {
        TransactionType::InvokeFunction => tx_type_label::INVOKE,
        TransactionType::Declare => tx_type_label::DECLARE,
        TransactionType::DeployAccount => tx_type_label::DEPLOY_ACCOUNT,
        TransactionType::L1Handler => tx_type_label::L1_HANDLER,
    }
}

/// Metrics for transaction execution operations.
#[derive(Debug)]
pub struct ExecutionMetrics {
    /// Histogram tracking per-transaction execution time in milliseconds.
    tx_execution_time_histogram: Histogram<f64>,
}

impl ExecutionMetrics {
    /// Register and initialize OTEL metrics.
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.exec.opentelemetry")
                .with_attributes([KeyValue::new("crate", "exec")])
                .build(),
        );

        let tx_execution_time_histogram = register_histogram_metric_instrument(
            &meter,
            "tx_execution_time_ms".to_string(),
            "Time taken to execute individual transactions".to_string(),
            "ms".to_string(),
        );

        Self { tx_execution_time_histogram }
    }

    /// Record transaction execution time with type and context labels.
    pub fn record_tx_execution_time(&self, duration_ms: f64, tx_type: &str, context: &str) {
        self.tx_execution_time_histogram.record(
            duration_ms,
            &[KeyValue::new("tx_type", tx_type.to_string()), KeyValue::new("context", context.to_string())],
        );
    }
}

/// Global metrics instance (initialized lazily on first access).
static METRICS: std::sync::LazyLock<ExecutionMetrics> = std::sync::LazyLock::new(ExecutionMetrics::register);

/// Get the global metrics instance.
pub fn metrics() -> &'static ExecutionMetrics {
    &METRICS
}

/// Helper to time transaction execution in RPC context.
pub struct TxExecutionTimer {
    start: Instant,
}

impl TxExecutionTimer {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    /// Finish timing and record metric with RPC context.
    pub fn finish(self, tx_type: TransactionType) {
        let duration_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        metrics().record_tx_execution_time(duration_ms, tx_type_to_label(tx_type), context_label::RPC);
    }
}

impl Default for TxExecutionTimer {
    fn default() -> Self {
        Self::new()
    }
}
