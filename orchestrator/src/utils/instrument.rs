use crate::types::params::OTELConfig;
use crate::utils::logging::{JsonEventFormatter, PrettyFormatter};
use crate::OrchestratorResult;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{SdkTracerProvider, Tracer};
use opentelemetry_sdk::Resource;
use std::time::Duration;
use tracing::{info, warn};
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
    /// setup - Initializes all the analytical instrumentation for the orchestrator
    ///
    /// # Arguments
    /// * `config` - The configuration for the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<OrchestratorInstrumentation>` - The instrumentation for the orchestrator.
    pub fn new(config: &OTELConfig) -> OrchestratorResult<Self> {
        match config.endpoint {
            None => {
                warn!(
                    service_name = %config.service_name,
                    "OTEL collector endpoint not configured. Telemetry export disabled. \
                    Set MADARA_ORCHESTRATOR_OTEL_COLLECTOR_ENDPOINT to enable metrics/traces/logs export."
                );
                Ok(Self { otel_config: config.clone(), meter_provider: None })
            }
            Some(ref endpoint) => {
                info!(
                    endpoint = %endpoint,
                    service_name = %config.service_name,
                    "Initializing OpenTelemetry exporters..."
                );

                let meter_provider = Self::instrument_metric_provider(config, endpoint)?;
                info!(
                    endpoint = %endpoint,
                    service_name = format!("{}_meter_service", config.service_name),
                    export_interval_secs = 5,
                    "OTEL metrics exporter initialized"
                );

                let tracer = Self::instrument_tracer_provider(config, endpoint)?;
                info!(
                    endpoint = %endpoint,
                    service_name = format!("{}_trace_service", config.service_name),
                    "OTEL tracer exporter initialized"
                );

                let logger = Self::instrument_logger_provider(config, endpoint)?;
                info!(
                    endpoint = %endpoint,
                    service_name = format!("{}_logs_service", config.service_name),
                    "OTEL logs exporter initialized"
                );

                // Respect LOG_FORMAT when composing the subscriber with OTEL layers
                let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "pretty".to_string());

                if log_format == "json" {
                    let subscriber = tracing_subscriber::registry()
                        .with(EnvFilter::from_default_env())
                        .with(
                            tracing_subscriber::fmt::layer()
                                .with_target(true)
                                .with_thread_ids(false)
                                .with_file(true)
                                .with_line_number(true)
                                .event_format(JsonEventFormatter),
                        )
                        .with(OpenTelemetryLayer::new(tracer))
                        .with(OpenTelemetryTracingBridge::new(&logger));
                    let _ = tracing::subscriber::set_global_default(subscriber);
                } else {
                    let subscriber = tracing_subscriber::registry()
                        .with(EnvFilter::from_default_env())
                        .with(
                            tracing_subscriber::fmt::layer()
                                .with_target(true)
                                .with_thread_ids(false)
                                .with_file(true)
                                .with_line_number(true)
                                .event_format(PrettyFormatter),
                        )
                        .with(OpenTelemetryLayer::new(tracer))
                        .with(OpenTelemetryTracingBridge::new(&logger));
                    let _ = tracing::subscriber::set_global_default(subscriber);
                }

                info!(
                    endpoint = %endpoint,
                    service_name = %config.service_name,
                    log_format = %log_format,
                    "OpenTelemetry fully initialized - metrics, traces, and logs exporters active"
                );

                Ok(Self { otel_config: config.clone(), meter_provider: Some(meter_provider) })
            }
        }
    }

    /// instrument_logs - This function sets up the logger provider for the orchestrator.
    /// It configures the logger to use the OpenTelemetry OTLP exporter
    /// and sets the resource attributes for the logger.
    ///
    /// # Arguments
    /// * `config` - The configuration for the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<LoggerProvider>` - The logger provider for the orchestrator.
    fn instrument_logger_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<SdkLoggerProvider> {
        let exporter =
            opentelemetry_otlp::LogExporterBuilder::new().with_tonic().with_endpoint(endpoint.to_string()).build()?;

        let logger_provider = SdkLoggerProvider::builder()
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![KeyValue::new(
                        opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                        format!("{}{}", config.service_name, "_logs_service"),
                    )])
                    .build(),
            )
            .with_batch_exporter(exporter)
            .build();

        Ok(logger_provider)
    }

    /// instrument_metric_provider - Instrumenting the metric provider for the system Analytics
    ///
    /// # Arguments
    /// * `config` - The configuration for the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<SdkMeterProvider>` - The meter provider for the orchestrator.
    fn instrument_metric_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<SdkMeterProvider> {
        let exporter = opentelemetry_otlp::MetricExporterBuilder::new()
            .with_tonic()
            .with_endpoint(endpoint.to_string())
            .build()?;

        let reader = PeriodicReader::builder(exporter).with_interval(Duration::from_secs(5)).build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![KeyValue::new(
                        opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                        format!("{}{}", config.service_name, "_meter_service"),
                    )])
                    .build(),
            )
            .build();

        global::set_meter_provider(provider.clone());
        Ok(provider)
    }
    /// instrument_tracer_provider - Instrumenting the tracer provider for the system Analytics
    ///
    /// # Arguments
    /// * `config` - The configuration for the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<Tracer>` - The tracer for the orchestrator.
    fn instrument_tracer_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<Tracer> {
        let exporter =
            opentelemetry_otlp::SpanExporterBuilder::new().with_tonic().with_endpoint(endpoint.to_string()).build()?;

        let resource = Resource::builder_empty()
            .with_attributes(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", config.service_name, "_trace_service"),
            )])
            .build();

        let provider = SdkTracerProvider::builder().with_resource(resource).with_batch_exporter(exporter).build();

        global::set_tracer_provider(provider.clone());

        Ok(provider.tracer(format!("{}{}", config.service_name, "_subscriber")))
    }

    /// shutdown - Shuts down the OpenTelemetry meter provider.
    /// This function is used to clean up resources and stop the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<()>` - A result indicating the success or failure of the shutdown operation.
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
