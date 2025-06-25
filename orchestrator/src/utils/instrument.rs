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
use tracing::warn;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
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

                // Force overwrite any existing global subscriber
                let _ = tracing::subscriber::set_global_default(subscriber);
                warn!("OpenTelemetry tracing subscriber initialized (existing subscriber overwritten if present)");
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
    fn instrument_logger_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<LoggerProvider> {
        Ok(opentelemetry_otlp::new_pipeline()
            .logging()
            .with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", config.service_name, "_logs_service"),
            )]))
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(endpoint.to_string()))
            .install_batch(runtime::Tokio)?)
    }

    /// instrument_metric_provider - Instrumenting the metric provider for the system Analytics
    ///
    /// # Arguments
    /// * `config` - The configuration for the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<SdkMeterProvider>` - The meter provider for the orchestrator.
    fn instrument_metric_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<SdkMeterProvider> {
        let export_config = ExportConfig { endpoint: endpoint.to_string(), ..ExportConfig::default() };
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
    ///
    /// # Arguments
    /// * `config` - The configuration for the OpenTelemetry exporter.
    ///
    /// # Returns
    /// * `OrchestratorResult<Tracer>` - The tracer for the orchestrator.
    fn instrument_tracer_provider(config: &OTELConfig, endpoint: &Url) -> OrchestratorResult<Tracer> {
        // Increasing the queue size and batch size, only increase in queue size delays full channel error.
        let batch_config = BatchConfigBuilder::default().build();

        let resource = Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            format!("{}{}", config.service_name, "_trace_service"),
        )]);

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(endpoint.to_string()))
            .with_trace_config(Config::default().with_resource(resource))
            .with_batch_config(batch_config)
            .install_batch(runtime::Tokio)?;

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
