use crate::types::params::OTELConfig;
use crate::OrchestratorResult;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{global, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::reader::{DefaultAggregationSelector, DefaultTemporalitySelector};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{BatchConfigBuilder, Config, Tracer};
use opentelemetry_sdk::{runtime, Resource};
use std::time::Duration;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Instrumentation for the Orchestrator
pub struct OrchestratorInstrumentation {
    pub otel_config: OTELConfig,
    pub meter_provider: SdkMeterProvider,
}

impl OrchestratorInstrumentation {
    /// setup - Initializes all the analytical instrumentation for the orchestrator
    pub fn setup(config: &OTELConfig) -> OrchestratorResult<Self> {
        let tracing_subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer()).with(EnvFilter::from_default_env());

        let meter_provider = Self::instrument_metric_provider(config)?;
        let tracer = Self::instrument_tracer_provider(config)?;
        let logger = Self::instrument_logs(config)?;

        tracing_subscriber.with(OpenTelemetryLayer::new(tracer)).with(OpenTelemetryTracingBridge::new(&logger)).init();
        Ok(Self { otel_config: config.clone(), meter_provider })
    }

    /// instrument_logs - instrumenting the logger for the orchestrator
    fn instrument_logs(config: &OTELConfig) -> OrchestratorResult<LoggerProvider> {
        Ok(opentelemetry_otlp::new_pipeline()
            .logging()
            .with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", config.service_name, "_logs_service"),
            )]))
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(config.endpoint.to_string()))
            .install_batch(runtime::Tokio)?)
    }

    /// instrument_metric_provider - Instrumenting the metric provider for the system Analytics
    fn instrument_metric_provider(config: &OTELConfig) -> OrchestratorResult<SdkMeterProvider> {
        let export_config = ExportConfig { endpoint: config.endpoint.to_string(), ..ExportConfig::default() };
        let exporter =
            opentelemetry_otlp::new_exporter().tonic().with_export_config(export_config).build_metrics_exporter(
                Box::new(DefaultAggregationSelector::new()),
                Box::new(DefaultTemporalitySelector::new()),
            )?;

        // Creates a periodic reader that exports every 5 seconds
        let reader = PeriodicReader::builder(exporter, runtime::Tokio).with_interval(Duration::from_secs(5)).build();

        // Builds a meter provider with the periodic reader
        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", config.service_name, "_meter_service"),
            )]))
            .build();
        global::set_meter_provider(provider.clone());
        Ok(provider)
    }
    /// instrument_tracer_provider - Instrumenting the tracer provider for the system Analytics
    fn instrument_tracer_provider(config: &OTELConfig) -> OrchestratorResult<Tracer> {
        // Increasing the queue size and batch size, only increase in queue size delays full channel error.
        let batch_config = BatchConfigBuilder::default().build();

        let resource = Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", config.service_name, "_trace_service"),
        )]);

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(config.endpoint.to_string()))
            .with_trace_config(Config::default().with_resource(resource))
            .with_batch_config(batch_config)
            .install_batch(runtime::Tokio)?;

        global::set_tracer_provider(provider.clone());

        Ok(provider.tracer(format!("{}{}", config.service_name, "_subscriber")))
    }

    pub fn shutdown(&self) -> OrchestratorResult<()> {
        global::shutdown_tracer_provider();
        Ok(self.meter_provider.shutdown()?)
    }
}
