use ::time::UtcOffset;
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
use std::fmt;
use std::fmt::Display;
use std::time::{Duration, SystemTime};
use time::{format_description, OffsetDateTime};
use tracing::field::{Field, Visit};
use tracing::Level;
use tracing_core::LevelFilter;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::EnvFilter;
use url::Url;

pub struct Analytics {
    meter_provider: Option<SdkMeterProvider>,
    service_name: String,
    collection_endpoint: Option<Url>,
}

impl Analytics {
    pub fn new(service_name: String, collection_endpoint: Option<Url>) -> anyhow::Result<Self> {
        Ok(Self { meter_provider: None, service_name, collection_endpoint })
    }

    pub fn setup(&mut self) -> anyhow::Result<()> {
        let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let custom_formatter = CustomFormatter { local_offset };

        let tracing_subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().event_format(custom_formatter).with_writer(std::io::stderr))
            .with(EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env()?);

        if self.collection_endpoint.is_none() {
            tracing_subscriber.init();
            return Ok(());
        };

        let tracer = self.init_tracer_provider()?;
        let logger_provider = self.init_logs()?;
        self.meter_provider = Some(self.init_metric_provider()?);

        let layer = OpenTelemetryTracingBridge::new(&logger_provider);
        tracing_subscriber.with(OpenTelemetryLayer::new(tracer)).with(layer).init();
        Ok(())
    }

    fn init_tracer_provider(&self) -> anyhow::Result<Tracer> {
        //  Guard clause if otel is disabled
        let otel_endpoint = self
            .collection_endpoint
            .clone()
            .ok_or(anyhow::anyhow!("OTEL endpoint is not set, not initializing otel providers."))?;

        let batch_config = BatchConfigBuilder::default().with_max_export_batch_size(128).build();

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

use tracing::Subscriber;
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
};

struct ValueVisitor {
    field_name: String,
    value: String,
}

impl Visit for ValueVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == self.field_name {
            self.value = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == self.field_name {
            self.value = value.to_string();
        }
    }
}

struct CustomFormatter {
    local_offset: UtcOffset,
}

impl<S, N> FormatEvent<S, N> for CustomFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let now = SystemTime::now();
        let datetime: OffsetDateTime = now.into();
        let local_datetime = datetime.to_offset(self.local_offset);
        let format =
            format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]:[subsecond digits:3]").unwrap();
        let ts = local_datetime.format(&format).unwrap();

        let brackets_style = console::Style::new().dim();

        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();

        let get_field_value = |field_name: &str| -> String {
            let mut visitor = ValueVisitor { field_name: field_name.to_string(), value: String::new() };
            event.record(&mut visitor);
            visitor.value.trim_matches('"').to_string()
        };

        match (level, target) {
            (&Level::INFO, "rpc_calls") => {
                let status = get_field_value("status");
                let method = get_field_value("method");
                let res_len = get_field_value("res_len");
                let response_time = get_field_value("response_time");

                let rpc_style = console::Style::new().magenta();

                let status_style =
                    if status == "200" { console::Style::new().green() } else { console::Style::new().red() };

                let time_style = if response_time.parse::<f64>().unwrap_or(0.0) <= 5.0 {
                    console::Style::new()
                } else {
                    console::Style::new().yellow()
                };

                writeln!(
                    writer,
                    "{} {} {} {} {} {} {} {} bytes - {} ms",
                    brackets_style.apply_to(format!("[{ts}]")),
                    brackets_style.apply_to(format!("[{level}]")),
                    brackets_style.apply_to("["),
                    rpc_style.apply_to("HTTP"),
                    brackets_style.apply_to("]"),
                    method,
                    status_style.apply_to(&status),
                    res_len,
                    time_style.apply_to(&response_time),
                )
            }
            (&Level::INFO, _) => {
                let message = get_field_value("message");

                writeln!(
                    writer,
                    "{} {} {}",
                    brackets_style.apply_to(format!("[{ts}]")),
                    brackets_style.apply_to(format!("[{level}]")),
                    message,
                )
            }
            (&Level::WARN, _) => {
                let message = get_field_value("message");
                writeln!(
                    writer,
                    "{} {} {}",
                    brackets_style.apply_to(format!("[{ts}]")),
                    brackets_style.apply_to(format!("[{level}]")),
                    message,
                )
            }
            (&Level::ERROR, "rpc_errors") => {
                let message = get_field_value("message");

                writeln!(
                    writer,
                    "{} {} {}",
                    brackets_style.apply_to(format!("[{ts}]")),
                    brackets_style.apply_to(format!("[{level}]")),
                    message,
                )
            }
            (&Level::ERROR, _) => {
                let message = get_field_value("message");
                writeln!(
                    writer,
                    "{} {} {}",
                    brackets_style.apply_to(format!("[{ts}]")),
                    brackets_style.apply_to(format!("[{level}]")),
                    message,
                )
            }
            _ => {
                let message = get_field_value("message");

                writeln!(
                    writer,
                    "{} {} {}",
                    brackets_style.apply_to(format!("[{ts}]")),
                    brackets_style.apply_to(format!("[{level}]")),
                    message,
                )
            }
        }
    }
}
