use crate::types::params::OTELConfig;
use crate::OrchestratorResult;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{global, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{BatchConfigBuilder, Config, Tracer};
use opentelemetry_sdk::{runtime, Resource};
use std::time::Duration;
use opentelemetry::logs::LoggerProvider;
use tracing::warn;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::EnvFilter;
use url::Url;

/// Instrumentation for the Orchestrator
pub struct OrchestratorInstrumentation {
    pub otel_config: OTELConfig,
    pub meter_provider: Option<SdkMeterProvider>,
}

impl OrchestratorInstrumentation {
    pub fn new(config: &OTELConfig) -> OrchestratorResult<Self> {
        match config.endpoint {
            None => {
                warn!("OTEL endpoint is not set. Skipping instrumentation.");
                Ok(Self { otel_config: config.clone(), meter_provider: None })
            }
            Some(ref endpoint) => {
                let tracing_subscriber = tracing_subscriber::registry()
                    .with(tracing_subscriber::fmt::layer())
                    .with(EnvFilter::from_default_env());

                let meter_provider = Self::instrument_metric_provider(config, endpoint)?;
                let tracer = Self::instrument_tracer_provider(config, endpoint)?;
                let logger = Self::instrument_logger_provider(config, endpoint)?;

                let subscriber = tracing_subscriber
                    .with(OpenTelemetryLayer::new(tracer))
                    .with(OpenTelemetryTracingBridge::new(&logger));

                let _ = tracing::subscriber::set_global_default(subscriber);
                warn!("OpenTelemetry tracing subscriber initialized (existing subscriber overwritten if present)");
                Ok(Self { otel_config: config.clone(), meter_provider: Some(meter_provider) })
            }
        }
    }

    fn instrument_logger_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<LoggerProvider> {
        Ok(opentelemetry_otlp::LogExporterBuilder::new()
            .logging()
            .with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", config.service_name, "_logs_service"),
            )]))
            .with_exporter(opentelemetry_otlp::LogExporterBuilder::new().tonic().with_endpoint(endpoint.to_string()))
            .install_batch(runtime::Tokio)?)
    }

    fn instrument_metric_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<SdkMeterProvider> {
        let export_config = ExportConfig { endpoint: Some(endpoint.to_string()), ..ExportConfig::default() };

        // exporter API changed: no more DefaultAggregationSelector/TemporalitySelector
        let exporter = opentelemetry_otlp::MetricExporterBuilder::new()
            .tonic()
            .with_export_config(export_config)
            .build_metrics_exporter()?;

        let reader = PeriodicReader::builder(exporter)
            .with_interval(Duration::from_secs(5))
            .build();

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

    fn instrument_tracer_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<Tracer> {
        let batch_config = BatchConfigBuilder::default().build();

        let resource = Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", config.service_name, "_trace_service"),
        )]);

        let provider = opentelemetry_otlp::SpanExporterBuilder::new()
            .tracing()
            .with_trace_config(Config::default().with_resource(resource))
            .with_batch_config(batch_config)
            .install_batch(runtime::Tokio)?;

        global::set_tracer_provider(provider.clone());
        Ok(provider.tracer(format!("{}{}", config.service_name, "_subscriber")))
    }

    pub fn shutdown(&self) -> OrchestratorResult<()> {
        match self.meter_provider {
            Some(ref meter_provider) => Ok(meter_provider.shutdown()?),
            None => {
                warn!("OTEL endpoint is not set. Skipping shutdown.");
                Ok(())
            }
        }
    }
}
