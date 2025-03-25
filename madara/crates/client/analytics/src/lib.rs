use formatter::CustomFormatter;
use mp_utils::service::{MadaraServiceId, Service, ServiceId, ServiceRunner};
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};
use opentelemetry::trace::TracerProvider;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use prometheus_endpoint::PrometheusEndpoint;
use std::fmt::Display;
use std::time::Duration;
use tracing_core::LevelFilter;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::EnvFilter;
use url::Url;

mod formatter;
mod prometheus_endpoint;

pub use prometheus_endpoint::PrometheusEndpointConfig;

#[derive(Debug, Clone)]
pub struct AnalyticsConfig {
    pub service_name: String,
    pub collection_endpoint: Option<Url>,
    pub prometheus_endpoint_config: PrometheusEndpointConfig,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            service_name: "Madara".into(),
            collection_endpoint: None,
            prometheus_endpoint_config: Default::default(),
        }
    }
}

#[derive(Clone)]
struct Providers {
    meter_provider: SdkMeterProvider,
    tracer_provider: SdkTracerProvider,
    logger_provider: SdkLoggerProvider,
}

pub struct AnalyticsService {
    providers: Option<Providers>,
    config: AnalyticsConfig,
    prometheus_endpoint: Option<PrometheusEndpoint>,
}

impl AnalyticsService {
    pub fn new(config: AnalyticsConfig) -> anyhow::Result<Self> {
        let prometheus_endpoint = if config.prometheus_endpoint_config.enabled {
            Some(PrometheusEndpoint::new(config.prometheus_endpoint_config.clone())?)
        } else {
            None
        };
        Ok(Self { providers: None, config, prometheus_endpoint })
    }

    pub fn setup(&mut self) -> anyhow::Result<()> {
        let tracing_subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer().event_format(CustomFormatter::new()), /* .with_writer(std::io::stderr) */
            )
            .with(EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env()?);

        if let Some(endpoint) = self.prometheus_endpoint.as_mut() {
            let exporter = endpoint.exporter.take().expect("Endpoint already setup");
            let provider = SdkMeterProvider::builder().with_reader(exporter).build();
            global::set_meter_provider(provider);
            tracing_subscriber.init();
            Ok(())
        } else {
            let Some(otel_endpoint) = &self.config.collection_endpoint else {
                tracing_subscriber.init();
                return Ok(());
            };

            let tracer_provider = self.init_tracer_provider(otel_endpoint)?;
            global::set_tracer_provider(tracer_provider.clone());

            let meter_provider = self.init_meter_provider(otel_endpoint)?;
            global::set_meter_provider(meter_provider.clone());

            let logger_provider = self.init_logs_provider(otel_endpoint)?;
            tracing_subscriber
                .with(OpenTelemetryLayer::new(
                    tracer_provider.tracer(format!("{}{}", self.config.service_name, "_subscriber")),
                ))
                .with(OpenTelemetryTracingBridge::new(&logger_provider))
                .init();

            self.providers = Some(Providers { meter_provider, tracer_provider, logger_provider });

            Ok(())
        }
    }

    fn init_tracer_provider(&self, otel_endpoint: &Url) -> anyhow::Result<SdkTracerProvider> {
        let exporter =
            opentelemetry_otlp::SpanExporter::builder().with_tonic().with_endpoint(otel_endpoint.clone()).build()?;

        let batch_config = BatchConfigBuilder::default().with_max_export_batch_size(128).build();

        let processor = BatchSpanProcessor::builder(exporter).with_batch_config(batch_config).build();

        let provider = SdkTracerProvider::builder()
            .with_span_processor(processor)
            .with_resource(
                Resource::builder()
                    .with_service_name(format!("{}{}", self.config.service_name, "_trace_service"))
                    .build(),
            )
            .build();

        Ok(provider)
    }

    fn init_meter_provider(&self, otel_endpoint: &Url) -> anyhow::Result<SdkMeterProvider> {
        let exporter =
            opentelemetry_otlp::MetricExporter::builder().with_tonic().with_endpoint(otel_endpoint.clone()).build()?;

        // Creates a periodic reader that exports every 5 seconds
        let reader = PeriodicReader::builder(exporter).with_interval(Duration::from_secs(5)).build();

        // Builds a meter provider with the periodic reader
        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(
                Resource::builder()
                    .with_service_name(format!("{}{}", self.config.service_name, "_meter_service"))
                    .build(),
            )
            .build();
        Ok(provider)
    }

    fn init_logs_provider(&self, otel_endpoint: &Url) -> anyhow::Result<SdkLoggerProvider> {
        let exporter =
            opentelemetry_otlp::LogExporter::builder().with_tonic().with_endpoint(otel_endpoint.clone()).build()?;

        Ok(SdkLoggerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name(format!("{}{}", self.config.service_name, "_logs_service"))
                    .build(),
            )
            .with_batch_exporter(exporter)
            .build())
    }
}

#[async_trait::async_trait]
impl Service for AnalyticsService {
    /// Default impl does not start any task.
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let mut endpoint = self.prometheus_endpoint.take();
        let providers = self.providers.take();
        runner.service_loop(move |mut ctx| async move {
            if let Some(endpoint) = endpoint.as_mut() {
                endpoint.run(ctx).await?;
            } else {
                ctx.cancelled().await;
            }

            if let Some(provider) = providers {
                provider.logger_provider.shutdown()?;
                provider.meter_provider.shutdown()?;
                provider.tracer_provider.shutdown()?;
            }
            anyhow::Ok(())
        });
        Ok(())
    }
}
impl ServiceId for AnalyticsService {
    #[inline(always)]
    fn svc_id(&self) -> mp_utils::service::PowerOfTwo {
        MadaraServiceId::Analytics.svc_id()
    }
}

pub trait GaugeType<T> {
    fn register_gauge(meter: &Meter, name: String, description: String, unit: String) -> Gauge<T>;
}

impl GaugeType<f64> for f64 {
    fn register_gauge(meter: &Meter, name: String, description: String, unit: String) -> Gauge<f64> {
        meter.f64_gauge(name).with_description(description).with_unit(unit).build()
    }
}
impl GaugeType<u64> for u64 {
    fn register_gauge(meter: &Meter, name: String, description: String, unit: String) -> Gauge<u64> {
        meter.u64_gauge(name).with_description(description).with_unit(unit).build()
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
        meter.u64_counter(name).with_description(description).with_unit(unit).build()
    }
}
impl CounterType<f64> for f64 {
    fn register_counter(meter: &Meter, name: String, description: String, unit: String) -> Counter<f64> {
        meter.f64_counter(name).with_description(description).with_unit(unit).build()
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
        meter.f64_histogram(name).with_description(description).with_unit(unit).build()
    }
}
impl HistogramType<u64> for u64 {
    fn register_histogram(meter: &Meter, name: String, description: String, unit: String) -> Histogram<u64> {
        meter.u64_histogram(name).with_description(description).with_unit(unit).build()
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
