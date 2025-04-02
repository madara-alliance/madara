// use std::time::Duration;
//
// use opentelemetry::trace::TracerProvider;
// use opentelemetry::{global, KeyValue};
// use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
// use opentelemetry_otlp::{ExportConfig, WithExportConfig};
// use opentelemetry_sdk::logs::LoggerProvider;
// use opentelemetry_sdk::metrics::reader::{DefaultAggregationSelector, DefaultTemporalitySelector};
// use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
// use opentelemetry_sdk::trace::{BatchConfigBuilder, Config, Tracer};
// use opentelemetry_sdk::{runtime, Resource};
// use tracing_opentelemetry::OpenTelemetryLayer;
// use tracing_subscriber::layer::SubscriberExt as _;
// use tracing_subscriber::util::SubscriberInitExt as _;
// use tracing_subscriber::EnvFilter;
// use url::Url;
// use crate::cli::instrumentation::InstrumentationCliArgs;
// use crate::{OrchestratorError, OrchestratorResult};
//
// #[derive(Debug, Clone)]
// pub struct InstrumentationParams {
//     pub otel_service_name: String,
//     pub otel_collector_endpoint: Option<Url>,
// }
//
// #[derive(Debug, Clone)]
// pub struct OTELConfig {
//     pub endpoint: Url,
//     pub service_name: String,
// }
//
// impl From<&InstrumentationCliArgs> for OTELConfig {
//     fn from(args: &InstrumentationCliArgs) -> OrchestratorResult<Self> {
//         let endpoint = args.otel_collector_endpoint.clone().ok_or_else(|e| OrchestratorError::FromDownstreamError(e))?;
//         let service_name = args.otel_service_name.clone().ok_or_else(|e| OrchestratorError::FromDownstreamError(e))?;
//         Ok(Self {
//             endpoint,
//             service_name,
//         })
//     }
//
// }
//
// from the instrumentation params, we can get the otel config
// impl From<InstrumentationParams> for OTELConfig {
//     fn from(instrumentation_params: InstrumentationParams) -> Self {
//         Self {
//             endpoint: instrumentation_params.otel_collector_endpoint.ok_or_else(|| "MADARA_ORCHESTRATOR_OTEL_COLLECTOR_ENDPOINT is not set".to_string()).unwrap(),
//             service_name: instrumentation_params.otel_service_name,
//         }
//     }
// }
//
// /// setup_analytics - Initializes all the analytical instrumentation for the orchestrator
// pub fn setup_analytics(otel_config: &OTELConfig) -> SdkMeterProvider {
//     let tracing_subscriber =
//         tracing_subscriber::registry().with(tracing_subscriber::fmt::layer()).with(EnvFilter::from_default_env());
//
//     let meter_provider = init_metric_provider(otel_config);
//     let tracer = init_tracer_provider(otel_config);
//     let logger_provider = init_logs(otel_config).unwrap();
//
//     // Create a new OpenTelemetryTracingBridge using the above LoggerProvider.
//     let layer = OpenTelemetryTracingBridge::new(&logger_provider);
//
//     tracing_subscriber.with(OpenTelemetryLayer::new(tracer)).with(layer).init();
//     meter_provider
// }
//
// TODO : we need to remove this since we have replace this with from trait
// fn get_otel_config(instrumentation: &InstrumentationParams) -> Option<OTELConfig> {
//     let otel_endpoint = instrumentation.otel_collector_endpoint.clone();
//     let otel_service_name = instrumentation.otel_service_name.clone();
//
//     match otel_endpoint {
//         Some(endpoint) => Some(OTELConfig { endpoint, service_name: otel_service_name }),
//         _ => {
//             tracing::warn!("MADARA_ORCHESTRATOR_OTEL_COLLECTOR_ENDPOINT is not set");
//             None
//         }
//     }
// }
//
// pub fn shutdown_analytics(meter_provider: SdkMeterProvider) {
//     global::shutdown_tracer_provider();
//     let _ = meter_provider.shutdown();
// }
//
// pub fn init_tracer_provider(otel_config: &OTELConfig) -> Tracer {
//     let batch_config = BatchConfigBuilder::default()
//         // Increasing the queue size and batch size, only increase in queue size delays full channel error.
//         .build();
//
//     let provider = opentelemetry_otlp::new_pipeline()
//         .tracing()
//         .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_config.endpoint.to_string()))
//         .with_trace_config(Config::default().with_resource(Resource::new(vec![KeyValue::new(
//             opentelemetry_semantic_conventions::resource::SERVICE_NAME,
//             format!("{}{}", otel_config.service_name, "_trace_service"),
//         )])))
//         .with_batch_config(batch_config)
//         .install_batch(runtime::Tokio)
//         .expect("Failed to install tracer provider");
//
//     global::set_tracer_provider(provider.clone());
//
//     provider.tracer(format!("{}{}", otel_config.service_name, "_subscriber"))
// }
//
// pub fn init_metric_provider(otel_config: &OTELConfig) -> SdkMeterProvider {
//     let export_config = ExportConfig { endpoint: otel_config.endpoint.to_string(), ..ExportConfig::default() };
//
//     // Creates and builds the OTLP exporter
//     let exporter = opentelemetry_otlp::new_exporter().tonic().with_export_config(export_config).build_metrics_exporter(
//         Box::new(DefaultAggregationSelector::new()),
//         Box::new(DefaultTemporalitySelector::new()),
//     );
//
//     // Creates a periodic reader that exports every 5 seconds
//     let reader = PeriodicReader::builder(exporter.expect("Failed to build metrics exporter"), runtime::Tokio)
//         .with_interval(Duration::from_secs(5))
//         .build();
//
//     // Builds a meter provider with the periodic reader
//     let provider = SdkMeterProvider::builder()
//         .with_reader(reader)
//         .with_resource(Resource::new(vec![KeyValue::new(
//             opentelemetry_semantic_conventions::resource::SERVICE_NAME,
//             format!("{}{}", otel_config.service_name, "_meter_service"),
//         )]))
//         .build();
//     global::set_meter_provider(provider.clone());
//     provider
// }
//
// fn init_logs(otel_config: &OTELConfig) -> Result<LoggerProvider, opentelemetry::logs::LogError> {
//     opentelemetry_otlp::new_pipeline()
//         .logging()
//         .with_resource(Resource::new(vec![KeyValue::new(
//             opentelemetry_semantic_conventions::resource::SERVICE_NAME,
//             format!("{}{}", otel_config.service_name, "_logs_service"),
//         )]))
//         .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_config.endpoint.to_string()))
//         .install_batch(runtime::Tokio)
// }
//
// #[cfg(test)]
// mod tests {
//     use once_cell::sync::Lazy;
//     use orchestrator_utils::metrics::lib::Metrics;
//     use orchestrator_utils::register_metric;
//     use crate::utils::metrics::OrchestratorMetrics;
//     use super::*;
//
//     #[tokio::test]
//     #[allow(clippy::needless_return)]
//     async fn test_init_metric_provider() {
//         // Set up necessary environment variables
//         let instrumentation_params = InstrumentationParams {
//             otel_collector_endpoint: Some(Url::parse("http://localhost:4317").unwrap()),
//             otel_service_name: "test_service".to_string(),
//         };
//
//         let otel_config = get_otel_config(&instrumentation_params).unwrap();
//
//         // Call the function and check if it doesn't panic
//         let result = std::panic::catch_unwind(|| {
//             let _provider = init_metric_provider(&otel_config);
//         });
//
//         // Check if the global meter provider is set
//         let _global_provider = global::meter_provider();
//         assert!(result.is_ok(), "init_metric_provider() panicked")
//     }
//
//     #[tokio::test]
//     #[allow(clippy::needless_return)]
//     async fn test_init_tracer_provider() {
//         // Set up necessary environment variables
//         let instrumentation_params = InstrumentationParams {
//             otel_collector_endpoint: Some(Url::parse("http://localhost:4317").unwrap()),
//             otel_service_name: "test_service".to_string(),
//         };
//
//         let otel_config = get_otel_config(&instrumentation_params).unwrap();
//
//         // Call the function and check if it doesn't panic
//         let result = std::panic::catch_unwind(|| {
//             let _tracer = init_tracer_provider(&otel_config);
//         });
//
//         assert!(result.is_ok(), "init_tracer_provider() panicked");
//     }
//
//     #[tokio::test]
//     #[allow(clippy::needless_return)]
//     async fn test_init_analytics() {
//         // This test just ensures that the function doesn't panic
//         let config = OTELConfig {
//             endpoint: Url::parse("http://localhost:4317").unwrap(),
//             service_name: "test_service".to_string(),
//         };
//
//         setup_analytics(&config);
//     }
//
//     #[tokio::test]
//     #[allow(clippy::needless_return)]
//     async fn test_gauge_setter() {
//         // This test just ensures that the function doesn't panic
//         let config = OTELConfig {
//             endpoint: Url::parse("http://localhost:4317").unwrap(),
//             service_name: "test_service".to_string(),
//         };
//
//         setup_analytics(&config);
//
//         register_metric!(ORCHESTRATOR_METRICS, OrchestratorMetrics);
//     }
// }
