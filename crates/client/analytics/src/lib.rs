use std::str::FromStr as _;
use std::time::Duration;

use mp_utils::service::Service;
use tokio::task::JoinSet;
use url::Url;

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

// Creating Open Telemetry resources
// Initializing Tracing
// Initializing Metrics
// Initializing Logging

#[derive(Debug, Clone)]
pub struct AnalyticsService {
    service_name: String,
    log_level: Level,
    collection_endpoint: Url,
}

impl AnalyticsService {
    pub fn new(service_name: String, log_level: String, collection_endpoint: Url) -> anyhow::Result<Self> {
        let log_level = Level::from_str(&log_level).unwrap_or(Level::INFO);
        Ok(Self { service_name, log_level, collection_endpoint })
    }

    fn setup(&mut self) -> anyhow::Result<()> {
        println!("Setting up analytics service");
        let otel_endpoint = self.collection_endpoint.clone();
        println!("OTEL endpoint: {}", otel_endpoint);
        let level = self.log_level;
        println!("Log level: {}", level);

        let tracing_subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::from_level(level))
            .with(tracing_subscriber::fmt::layer().with_target(false));
        println!("Tracing subscriber:");

        if !otel_endpoint.host_str().unwrap().is_empty() {
            let _meter_provider = self.init_metric_provider();
            let tracer = self.init_tracer_provider();
            println!("Tracer: {}", "hi");

            // Opentelemetry will not provide a global API to manage the logger
            // provider. Application users must manage the lifecycle of the logger
            // provider on their own. Dropping logger providers will disable log
            // emitting.

            let logger_provider = self.init_logs().unwrap();
            println!("Logger provider");
            // Create a new OpenTelemetryTracingBridge using the above LoggerProvider.
            let layer = OpenTelemetryTracingBridge::new(&logger_provider);
            println!("Layer: ");

            tracing_subscriber.with(OpenTelemetryLayer::new(tracer)).with(layer).init();
            println!("Analytics service setup complete");
            Ok(())
        } else {
            println!("OTEL endpoint is empty");
            tracing_subscriber.init();
            println!("Analytics basic service setup complete");
            Ok(())
        }
    }

    fn init_tracer_provider(&self) -> Tracer {
        let batch_config = BatchConfigBuilder::default()
    // Increasing the queue size and batch size, only increase in queue size delays full channel error.
    .build();

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter().tonic().with_endpoint(self.collection_endpoint.to_string()),
            )
            .with_trace_config(Config::default().with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", self.service_name, "_trace_service"),
            )])))
            .with_batch_config(batch_config)
            .install_batch(runtime::Tokio)
            .expect("Failed to install tracer provider");

        global::set_tracer_provider(provider.clone());
        provider.tracer(format!("{}{}", self.service_name, "_subscriber"))
    }

    fn init_metric_provider(&self) -> SdkMeterProvider {
        let export_config = ExportConfig { endpoint: self.collection_endpoint.to_string(), ..ExportConfig::default() };

        // Creates and builds the OTLP exporter
        let exporter =
            opentelemetry_otlp::new_exporter().tonic().with_export_config(export_config).build_metrics_exporter(
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
                format!("{}{}", self.service_name, "_meter_service"),
            )]))
            .build();
        global::set_meter_provider(provider.clone());
        provider
    }

    fn init_logs(&self) -> Result<LoggerProvider, opentelemetry::logs::LogError> {
        opentelemetry_otlp::new_pipeline()
            .logging()
            .with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", self.service_name, "_logs_service"),
            )]))
            .with_exporter(
                opentelemetry_otlp::new_exporter().tonic().with_endpoint(self.collection_endpoint.to_string()),
            )
            .install_batch(runtime::Tokio)
    }

    // fn shutdown(&self, meter_provider: Option<SdkMeterProvider>) {
    //     let otel_endpoint = get_env_var_or_panic("OTEL_COLLECTOR_ENDPOINT");

    //     // guard clause if otel is disabled
    //     if otel_endpoint.is_empty() {
    //         return;
    //     }

    //     if let Some(meter_provider) = meter_provider {
    //         global::shutdown_tracer_provider();
    //         let _ = meter_provider.shutdown();
    //     }
    // }
}

#[async_trait::async_trait]
impl Service for AnalyticsService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        self.setup()?;
        join_set.spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            Ok(())
        });
        Ok(())
    }
}

// Utils
use opentelemetry::metrics::{Counter, Gauge, Meter};

pub trait Metrics {
    fn register() -> Self;
}

#[macro_export]
macro_rules! register_metric {
    ($name:ident, $type:ty) => {
        pub static $name: Lazy<$type> = Lazy::new(|| <$type>::register());
    };
}

pub fn register_gauge_metric_instrument(
    crate_meter: &Meter,
    instrument_name: String,
    desc: String,
    unit: String,
) -> Gauge<u64> {
    crate_meter.u64_gauge(instrument_name).with_description(desc).with_unit(unit).init()
}

pub fn register_counter_metric_instrument(
    crate_meter: &Meter,
    instrument_name: String,
    desc: String,
    unit: String,
) -> Counter<u64> {
    crate_meter.u64_counter(instrument_name).with_description(desc).with_unit(unit).init()
}
