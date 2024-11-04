use std::str::FromStr;
use std::time::Duration;

use opentelemetry::trace::TracerProvider;
use opentelemetry::{global, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::reader::{DefaultAggregationSelector, DefaultTemporalitySelector};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{BatchConfigBuilder, Config, Tracer};
use opentelemetry_sdk::{runtime, Resource};
use tracing::Level;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::EnvFilter;
use utils::env_utils::{get_env_var_optional, get_env_var_or_default};

pub struct OTELConfig {
    endpoint: String,
    service_name: String,
}

pub fn setup_analytics() -> Option<SdkMeterProvider> {
    let otel_config = get_otel_config();

    let log_level = get_env_var_or_default("RUST_LOG", "INFO");
    let level = Level::from_str(&log_level).unwrap_or(Level::INFO);

    let tracing_subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(level))
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env());

    if let Some(otel_config) = otel_config {
        let meter_provider = init_metric_provider(&otel_config);
        let tracer = init_tracer_provider(&otel_config);

        // Opentelemetry will not provide a global API to manage the logger
        // provider. Application users must manage the lifecycle of the logger
        // provider on their own. Dropping logger providers will disable log
        // emitting.
        let logger_provider = init_logs(&otel_config).unwrap();
        // Create a new OpenTelemetryTracingBridge using the above LoggerProvider.
        let layer = OpenTelemetryTracingBridge::new(&logger_provider);

        tracing_subscriber.with(OpenTelemetryLayer::new(tracer)).with(layer).init();
        Some(meter_provider)
    } else {
        tracing_subscriber.init();
        None
    }
}

fn get_otel_config() -> Option<OTELConfig> {
    let otel_endpoint = get_env_var_optional("OTEL_COLLECTOR_ENDPOINT").expect("Failed to get OTEL_COLLECTOR_ENDPOINT");
    let otel_service_name = get_env_var_optional("OTEL_SERVICE_NAME").expect("Failed to get OTEL_SERVICE_NAME");

    match (otel_endpoint, otel_service_name) {
        (Some(endpoint), Some(service_name)) => Some(OTELConfig { endpoint, service_name }),
        _ => {
            tracing::warn!("OTEL_COLLECTOR_ENDPOINT or OTEL_SERVICE_NAME is not set");
            None
        }
    }
}

pub fn shutdown_analytics(meter_provider: Option<SdkMeterProvider>) {
    let otel_config = get_otel_config();

    // guard clause if otel is disabled
    if otel_config.is_none() {
        return;
    }

    if let Some(meter_provider) = meter_provider {
        global::shutdown_tracer_provider();
        let _ = meter_provider.shutdown();
    }
}

pub fn init_tracer_provider(otel_config: &OTELConfig) -> Tracer {
    let batch_config = BatchConfigBuilder::default()
    // Increasing the queue size and batch size, only increase in queue size delays full channel error.
    .build();

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_config.endpoint.to_string()))
        .with_trace_config(Config::default().with_resource(Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", otel_config.service_name, "_trace_service"),
        )])))
        .with_batch_config(batch_config)
        .install_batch(runtime::Tokio)
        .expect("Failed to install tracer provider");

    global::set_tracer_provider(provider.clone());

    provider.tracer(format!("{}{}", otel_config.service_name, "_subscriber"))
}

pub fn init_metric_provider(otel_config: &OTELConfig) -> SdkMeterProvider {
    let export_config = ExportConfig { endpoint: otel_config.endpoint.to_string(), ..ExportConfig::default() };

    // Creates and builds the OTLP exporter
    let exporter = opentelemetry_otlp::new_exporter().tonic().with_export_config(export_config).build_metrics_exporter(
        // TODO: highly likely that changing these configs will result in correct collection of traces, inhibiting full
        // channel issue
        Box::new(DefaultAggregationSelector::new()),
        Box::new(DefaultTemporalitySelector::new()),
    );

    // Creates a periodic reader that exports every 5 seconds
    let reader = PeriodicReader::builder(exporter.expect("Failed to build metrics exporter"), runtime::Tokio)
        .with_interval(Duration::from_secs(5))
        .build();

    // Builds a meter provider with the periodic reader
    let provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", otel_config.service_name, "_meter_service"),
        )]))
        .build();
    global::set_meter_provider(provider.clone());
    provider
}

fn init_logs(otel_config: &OTELConfig) -> Result<LoggerProvider, opentelemetry::logs::LogError> {
    opentelemetry_otlp::new_pipeline()
        .logging()
        .with_resource(Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", otel_config.service_name, "_logs_service"),
        )]))
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_config.endpoint.to_string()))
        .install_batch(runtime::Tokio)
}

#[cfg(test)]
mod tests {
    use std::env;

    use once_cell::sync::Lazy;
    use utils::metrics::lib::Metrics;
    use utils::register_metric;

    use super::*;
    use crate::metrics::OrchestratorMetrics;

    #[tokio::test]
    #[allow(clippy::needless_return)]
    async fn test_init_metric_provider() {
        // Set up necessary environment variables
        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("OTEL_SERVICE_NAME", "test_service");

        let otel_config = get_otel_config().unwrap();

        // Call the function and check if it doesn't panic
        let result = std::panic::catch_unwind(|| {
            let _provider = init_metric_provider(&otel_config);
        });

        // Check if the global meter provider is set
        let _global_provider = global::meter_provider();
        assert!(result.is_ok(), "init_metric_provider() panicked")
    }

    #[tokio::test]
    #[allow(clippy::needless_return)]
    async fn test_init_tracer_provider() {
        // Set up necessary environment variables
        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("OTEL_SERVICE_NAME", "test_service");
        let otel_config = get_otel_config().unwrap();

        // Call the function and check if it doesn't panic
        let result = std::panic::catch_unwind(|| {
            let _tracer = init_tracer_provider(&otel_config);
        });

        assert!(result.is_ok(), "init_tracer_provider() panicked");
    }

    #[tokio::test]
    #[allow(clippy::needless_return)]
    async fn test_init_analytics() {
        // This test just ensures that the function doesn't panic

        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("OTEL_SERVICE_NAME", "test_service");

        let analytics = setup_analytics();

        assert!(analytics.is_some(), " Unable to set analytics")
    }

    #[tokio::test]
    #[allow(clippy::needless_return)]
    async fn test_gauge_setter() {
        // This test just ensures that the function doesn't panic

        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("OTEL_SERVICE_NAME", "test_service");

        setup_analytics();

        register_metric!(ORCHESTRATOR_METRICS, OrchestratorMetrics);
    }
}
