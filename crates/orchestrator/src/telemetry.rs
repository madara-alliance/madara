use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_otlp::ExportConfig;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::reader::DefaultAggregationSelector;
use opentelemetry_sdk::metrics::reader::DefaultTemporalitySelector;
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::BatchConfigBuilder;
use opentelemetry_sdk::trace::Config;
use opentelemetry_sdk::trace::Tracer;
use opentelemetry_sdk::{runtime, Resource};
use std::str::FromStr as _;
use std::time::Duration;
use tracing::Level;

use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use utils::env_utils::get_env_var_or_panic;

use lazy_static::lazy_static;

lazy_static! {
    #[derive(Debug)]
    pub static ref OTEL_SERVICE_NAME: String = get_env_var_or_panic("OTEL_SERVICE_NAME");
}

pub fn setup_analytics() -> Option<SdkMeterProvider> {
    let otel_endpoint = get_env_var_or_panic("OTEL_COLLECTOR_ENDPOINT");
    let tracing_level = Level::from_str(&get_env_var_or_panic("TRACING_LEVEL"))
        .expect("Could not obtain tracing level from environment variable.");

    // guard clause if otel is disabled
    if otel_endpoint.is_empty() {
        return None;
    }

    let meter_provider = init_metric_provider(otel_endpoint.as_str());
    let tracer = init_tracer_provider(otel_endpoint.as_str());

    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(tracing_level))
        .with(tracing_subscriber::fmt::layer())
        .with(OpenTelemetryLayer::new(tracer))
        .init();

    Some(meter_provider)
}

pub fn shutdown_analytics(meter_provider: Option<SdkMeterProvider>) {
    let otel_endpoint = get_env_var_or_panic("OTEL_COLLECTOR_ENDPOINT");

    // guard clause if otel is disabled
    if otel_endpoint.is_empty() {
        return;
    }

    if let Some(meter_provider) = meter_provider {
        global::shutdown_tracer_provider();
        let _ = meter_provider.shutdown();
    }
}

pub fn init_tracer_provider(otel_endpoint: &str) -> Tracer {
    let batch_config = BatchConfigBuilder::default()
    // Increasing the queue size and batch size, only increase in queue size delays full channel error.
    .build();

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_endpoint.to_string()))
        .with_trace_config(Config::default().with_resource(Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", *OTEL_SERVICE_NAME, "_trace_service"),
        )])))
        .with_batch_config(batch_config)
        .install_batch(runtime::Tokio)
        .unwrap();

    global::set_tracer_provider(provider.clone());

    provider.tracer(format!("{}{}", *OTEL_SERVICE_NAME, "_subscriber"))
}

pub fn init_metric_provider(otel_endpoint: &str) -> SdkMeterProvider {
    let export_config = ExportConfig { endpoint: otel_endpoint.to_string(), ..ExportConfig::default() };

    // Creates and builds the OTLP exporter
    let exporter = opentelemetry_otlp::new_exporter().tonic().with_export_config(export_config).build_metrics_exporter(
        // TODO: highly likely that changing these configs will result in correct collection of traces, inhibiting full channel issue
        Box::new(DefaultAggregationSelector::new()),
        Box::new(DefaultTemporalitySelector::new()),
    );

    // Creates a periodic reader that exports every 5 seconds
    let reader =
        PeriodicReader::builder(exporter.unwrap(), runtime::Tokio).with_interval(Duration::from_secs(5)).build();

    // Builds a meter provider with the periodic reader
    let provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", *OTEL_SERVICE_NAME, "_meter_service"),
        )]))
        .build();
    global::set_meter_provider(provider.clone());
    provider
}

#[cfg(test)]
mod tests {
    use crate::metrics::OrchestratorMetrics;
    use once_cell::sync::Lazy;
    use utils::{metrics::lib::Metrics, register_metric};

    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_init_metric_provider() {
        // Set up necessary environment variables
        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("TRACING_LEVEL", "info");
        env::set_var("OTEL_SERVICE_NAME", "test_service");

        let otel_endpoint = get_env_var_or_panic("OTEL_COLLECTOR_ENDPOINT");

        // Call the function and check if it doesn't panic
        let result = std::panic::catch_unwind(|| {
            let _provider = init_metric_provider(&otel_endpoint);
        });

        // Check if the global meter provider is set
        let _global_provider = global::meter_provider();
        assert!(result.is_ok(), "init_metric_provider() panicked");
    }

    #[tokio::test]
    async fn test_init_tracer_provider() {
        // Set up necessary environment variables
        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("TRACING_LEVEL", "info");
        env::set_var("OTEL_SERVICE_NAME", "test_service");
        let otel_endpoint = get_env_var_or_panic("OTEL_COLLECTOR_ENDPOINT");

        // Call the function and check if it doesn't panic
        let result = std::panic::catch_unwind(|| {
            let _tracer = init_tracer_provider(otel_endpoint.as_str());
        });

        assert!(result.is_ok(), "init_tracer_provider() panicked");
    }

    #[tokio::test]
    async fn test_init_analytics() {
        // This test just ensures that the function doesn't panic

        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("TRACING_LEVEL", "info");
        env::set_var("OTEL_SERVICE_NAME", "test_service");

        let analytics = setup_analytics();

        assert!(analytics.is_some(), " Unable to set analytics")
    }

    #[tokio::test]
    async fn test_gauge_setter() {
        // This test just ensures that the function doesn't panic

        env::set_var("OTEL_COLLECTOR_ENDPOINT", "http://localhost:4317");
        env::set_var("TRACING_LEVEL", "info");
        env::set_var("OTEL_SERVICE_NAME", "test_service");

        setup_analytics();

        register_metric!(ORCHESTRATOR_METRICS, OrchestratorMetrics);
    }
}
