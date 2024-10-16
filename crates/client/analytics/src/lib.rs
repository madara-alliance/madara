use opentelemetry::trace::TracerProvider;
use opentelemetry::{global, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::reader::{DefaultAggregationSelector, DefaultTemporalitySelector};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{BatchConfigBuilder, Config, Tracer};
use opentelemetry_sdk::{runtime, Resource};
use std::str::FromStr as _;
use std::time::Duration;
use tracing::Level;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;

#[derive(Debug, Clone)]
pub struct Analytics {
    meter_provider: Option<SdkMeterProvider>,
    service_name: String,
    log_level: Level,
    collection_endpoint: Url,
}

impl Analytics {
    pub fn new(service_name: String, log_level: String, collection_endpoint: Url) -> anyhow::Result<Self> {
        let log_level = Level::from_str(&log_level).unwrap_or(Level::INFO);
        Ok(Self { meter_provider: None, service_name, log_level, collection_endpoint })
    }

    pub fn setup(&mut self) -> anyhow::Result<()> {
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
            let meter_provider = self.init_metric_provider();
            self.meter_provider = Some(meter_provider);
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

    pub fn shutdown(&self) {
        // guard clause if otel is disabled
        if self.collection_endpoint.to_string().is_empty() {
            return;
        }

        if let Some(meter_provider) = self.meter_provider.clone() {
            global::shutdown_tracer_provider();
            let _ = meter_provider.shutdown();
        }
    }
}

// Utils
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

use std::fmt::Display;

// TODO: this could be optimized by using generics
pub trait GaugeType<T> {
    fn register_gauge(meter: &Meter, name: String, description: String, unit: String) -> Gauge<T>;
}

impl GaugeType<f64> for f64 {
    fn register_gauge(meter: &Meter, name: String, description: String, unit: String) -> Gauge<f64> {
        meter.f64_gauge(name).with_description(description).with_unit(unit).init()
    }
}

impl GaugeType<u64> for u64 {
    fn register_gauge(meter: &Meter, name: String, description: String, unit: String) -> Gauge<u64> {
        meter.u64_gauge(name).with_description(description).with_unit(unit).init()
    }
}

pub fn register_gauge_metric_instrument<T: GaugeType<T> + Display>(
    crate_meter: &Meter,
    instrument_name: String,
    desc: String,
    unit: String,
) -> Gauge<T> {
    T::register_gauge(crate_meter, instrument_name, desc, unit)
}

pub trait CounterType<T> {
    fn register_counter(meter: &Meter, name: String, description: String, unit: String) -> Counter<T>;
}

impl CounterType<u64> for u64 {
    fn register_counter(meter: &Meter, name: String, description: String, unit: String) -> Counter<u64> {
        meter.u64_counter(name).with_description(description).with_unit(unit).init()
    }
}
impl CounterType<f64> for f64 {
    fn register_counter(meter: &Meter, name: String, description: String, unit: String) -> Counter<f64> {
        meter.f64_counter(name).with_description(description).with_unit(unit).init()
    }
}
pub fn register_counter_metric_instrument<T: CounterType<T> + Display>(
    crate_meter: &Meter,
    instrument_name: String,
    desc: String,
    unit: String,
) -> Counter<T> {
    T::register_counter(crate_meter, instrument_name, desc, unit)
}

pub trait HistogramType<T> {
    fn register_histogram(meter: &Meter, name: String, description: String, unit: String) -> Histogram<T>;
}

impl HistogramType<f64> for f64 {
    fn register_histogram(meter: &Meter, name: String, description: String, unit: String) -> Histogram<f64> {
        meter.f64_histogram(name).with_description(description).with_unit(unit).init()
    }
}

impl HistogramType<u64> for u64 {
    fn register_histogram(meter: &Meter, name: String, description: String, unit: String) -> Histogram<u64> {
        meter.u64_histogram(name).with_description(description).with_unit(unit).init()
    }
}

pub fn register_histogram_metric_instrument<T: HistogramType<T> + Display>(
    crate_meter: &Meter,
    instrument_name: String,
    desc: String,
    unit: String,
) -> Histogram<T> {
    T::register_histogram(crate_meter, instrument_name, desc, unit)
}
