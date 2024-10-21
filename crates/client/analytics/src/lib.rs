use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};
use opentelemetry::trace::TracerProvider;
use opentelemetry::{global, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{ExportConfig, WithExportConfig};
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::reader::{DefaultAggregationSelector, DefaultTemporalitySelector};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{BatchConfigBuilder, Config, Tracer};
use opentelemetry_sdk::{runtime, Resource};
use std::fmt::Display;
use std::time::Duration;
use tracing::Level;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;

pub struct Analytics {
    meter_provider: Option<SdkMeterProvider>,
    service_name: String,
    log_level: Level,
    collection_endpoint: Option<Url>,
}

impl Analytics {
    pub fn new(
        service_name: String,
        log_level: tracing::Level,
        collection_endpoint: Option<Url>,
    ) -> anyhow::Result<Self> {
        Ok(Self { meter_provider: None, service_name, log_level, collection_endpoint })
    }

    pub fn setup(&mut self) -> anyhow::Result<()> {
        let format = tracing_subscriber::fmt::format()
            .pretty()
            .with_level(true)
            .with_target(true)
            .with_thread_names(true)
            .with_thread_ids(true)
            .with_file(false)
            .with_timer(tracing_subscriber::fmt::time::SystemTime)
            .with_ansi(true);

        let tracing_subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::from_level(self.log_level))
            .with(tracing_subscriber::fmt::layer().event_format(format));

        if self.collection_endpoint.is_none() {
            tracing_subscriber.init();
            return Ok(());
        };

        let tracer = self.init_tracer_provider()?;
        let logger_provider = self.init_logs()?;
        self.meter_provider = Some(self.init_metric_provider()?);

        let layer = OpenTelemetryTracingBridge::new(&logger_provider);
        tracing_subscriber.with(OpenTelemetryLayer::new(tracer)).with(layer).init();

        tracing::info!("OTEL initialized");
        Ok(())
    }

    fn init_tracer_provider(&self) -> anyhow::Result<Tracer> {
        //  Guard clause if otel is disabled
        let otel_endpoint = self
            .collection_endpoint
            .clone()
            .ok_or(anyhow::anyhow!("OTEL endpoint is not set, not initializing otel providers."))?;

        let batch_config = BatchConfigBuilder::default().build();

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_endpoint.to_string()))
            .with_trace_config(Config::default().with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", self.service_name, "_trace_service"),
            )])))
            .with_batch_config(batch_config)
            .install_batch(runtime::Tokio)
            .expect("Failed to install tracer provider");

        global::set_tracer_provider(provider.clone());
        Ok(provider.tracer(format!("{}{}", self.service_name, "_subscriber")))
    }

    fn init_metric_provider(&self) -> anyhow::Result<SdkMeterProvider> {
        //  Guard clause if otel is disabled
        let otel_endpoint = self
            .collection_endpoint
            .clone()
            .ok_or(anyhow::anyhow!("OTEL endpoint is not set, not initializing otel providers."))?;

        let export_config = ExportConfig { endpoint: otel_endpoint.to_string(), ..ExportConfig::default() };

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
        Ok(provider)
    }

    fn init_logs(&self) -> anyhow::Result<LoggerProvider> {
        //  Guard clause if otel is disabled
        let otel_endpoint = self
            .collection_endpoint
            .clone()
            .ok_or(anyhow::anyhow!("OTEL endpoint is not set, not initializing otel providers."))?;

        let logger = opentelemetry_otlp::new_pipeline()
            .logging()
            .with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                format!("{}{}", self.service_name, "_logs_service"),
            )]))
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(otel_endpoint.to_string()))
            .install_batch(runtime::Tokio)?;

        Ok(logger)
    }

    pub fn shutdown(&self) -> anyhow::Result<()> {
        // guard clause if otel is disabled
        if self.collection_endpoint.is_none() {
            return Ok(());
        }

        if let Some(meter_provider) = self.meter_provider.clone() {
            global::shutdown_tracer_provider();
            let _ = meter_provider.shutdown();
        }

        Ok(())
    }
}

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
