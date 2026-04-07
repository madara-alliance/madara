//! OpenTelemetry service and metric registration helpers for the Madara node.

use anyhow::Context;
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
use std::{env, fmt::Display, time::Duration};
use tracing_core::LevelFilter;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::EnvFilter;
use url::Url;

mod formatter;
mod sysinfo;

pub use sysinfo::*;

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Logical name reported as `service.name` to the OTEL collector. Used as
    /// the prefix for the per-signal service names (e.g. `madara` becomes
    /// `madara_meter_service`, `madara_trace_service`, `madara_logs_service`).
    pub service_name: String,
    /// OTLP/gRPC endpoint of the OTEL collector to export to. When `None`, the
    /// service falls back to the standard `OTEL_EXPORTER_OTLP_ENDPOINT`
    /// environment variable; if that is also unset, no OTEL exporters are
    /// installed and only the local `tracing_subscriber` is configured.
    pub collection_endpoint: Option<Url>,
    /// Whether to install a metric exporter and register a global
    /// `SdkMeterProvider`. Has no effect when `collection_endpoint` resolves to
    /// `None`.
    pub export_metrics: bool,
    /// Whether to install a span exporter and register a global
    /// `SdkTracerProvider`. Has no effect when `collection_endpoint` resolves
    /// to `None`.
    pub export_traces: bool,
    /// Whether to install a log exporter and bridge `tracing` events into OTEL
    /// logs via `OpenTelemetryTracingBridge`. Has no effect when
    /// `collection_endpoint` resolves to `None`.
    pub export_logs: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "madara".into(),
            collection_endpoint: None,
            export_metrics: true,
            export_traces: false,
            export_logs: false,
        }
    }
}

#[derive(Clone)]
struct Providers {
    meter_provider: Option<SdkMeterProvider>,
    tracer_provider: Option<SdkTracerProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

/// `TelemetryService` is intentionally **non-restartable**: it owns the global
/// OTEL providers (`SdkMeterProvider`, `SdkTracerProvider`, `SdkLoggerProvider`)
/// which are installed once into `opentelemetry::global` during `setup()` and
/// must only be shut down once. The admin RPC service surface explicitly
/// rejects start/stop/restart of `MadaraServiceId::Telemetry` via
/// `ensure_services_are_externally_controllable` (see
/// `madara/crates/client/rpc/src/versions/admin/v0_1_0/methods/services.rs`).
/// If you ever re-enable external control over this service, you must also
/// rework the `Service::start` impl below — currently `providers.take()`
/// assumes a single call.
pub struct TelemetryService {
    providers: Option<Providers>,
    config: TelemetryConfig,
}

impl TelemetryService {
    pub fn new(config: TelemetryConfig) -> anyhow::Result<Self> {
        Ok(Self { providers: None, config })
    }

    fn subscriber_service_name(&self) -> String {
        format!("{}_subscriber", self.config.service_name)
    }

    fn tracer_service_name(&self) -> String {
        format!("{}_trace_service", self.config.service_name)
    }

    fn meter_service_name(&self) -> String {
        format!("{}_meter_service", self.config.service_name)
    }

    fn logger_service_name(&self) -> String {
        format!("{}_logs_service", self.config.service_name)
    }

    fn collection_endpoint(&self) -> anyhow::Result<Option<Url>> {
        if let Some(endpoint) = &self.config.collection_endpoint {
            return Ok(Some(endpoint.clone()));
        }

        let Some(endpoint) = env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT") else {
            return Ok(None);
        };
        let endpoint =
            endpoint.into_string().map_err(|_| anyhow::anyhow!("OTEL_EXPORTER_OTLP_ENDPOINT must be valid UTF-8"))?;

        Url::parse(&endpoint)
            .with_context(|| format!("Parsing OTEL_EXPORTER_OTLP_ENDPOINT as a URL: {endpoint}"))
            .map(Some)
    }

    pub fn setup(&mut self) -> anyhow::Result<()> {
        let tracing_subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().event_format(CustomFormatter::new()))
            .with(EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env()?);

        let Some(otel_endpoint) = self.collection_endpoint()? else {
            tracing_subscriber.init();
            return Ok(());
        };

        let meter_provider = self
            .config
            .export_metrics
            .then(|| -> anyhow::Result<SdkMeterProvider> {
                let provider = self.init_meter_provider(&otel_endpoint)?;
                global::set_meter_provider(provider.clone());
                Ok(provider)
            })
            .transpose()?;

        let tracer_provider = self
            .config
            .export_traces
            .then(|| -> anyhow::Result<SdkTracerProvider> {
                let provider = self.init_tracer_provider(&otel_endpoint)?;
                global::set_tracer_provider(provider.clone());
                Ok(provider)
            })
            .transpose()?;

        let logger_provider = self
            .config
            .export_logs
            .then(|| -> anyhow::Result<SdkLoggerProvider> { self.init_logs_provider(&otel_endpoint) })
            .transpose()?;

        match (&tracer_provider, &logger_provider) {
            (Some(tracer), Some(logger)) => {
                tracing_subscriber
                    .with(OpenTelemetryLayer::new(tracer.tracer(self.subscriber_service_name())))
                    .with(OpenTelemetryTracingBridge::new(logger))
                    .init();
            }
            (Some(tracer), None) => {
                tracing_subscriber.with(OpenTelemetryLayer::new(tracer.tracer(self.subscriber_service_name()))).init();
            }
            (None, Some(logger)) => {
                tracing_subscriber.with(OpenTelemetryTracingBridge::new(logger)).init();
            }
            (None, None) => {
                tracing_subscriber.init();
            }
        }

        self.providers = Some(Providers { meter_provider, tracer_provider, logger_provider });

        Ok(())
    }

    fn init_tracer_provider(&self, otel_endpoint: &Url) -> anyhow::Result<SdkTracerProvider> {
        let exporter =
            opentelemetry_otlp::SpanExporter::builder().with_tonic().with_endpoint(otel_endpoint.clone()).build()?;

        let batch_config = BatchConfigBuilder::default().with_max_export_batch_size(128).build();
        let processor = BatchSpanProcessor::builder(exporter).with_batch_config(batch_config).build();

        Ok(SdkTracerProvider::builder()
            .with_span_processor(processor)
            .with_resource(Resource::builder().with_service_name(self.tracer_service_name()).build())
            .build())
    }

    fn init_meter_provider(&self, otel_endpoint: &Url) -> anyhow::Result<SdkMeterProvider> {
        let exporter =
            opentelemetry_otlp::MetricExporter::builder().with_tonic().with_endpoint(otel_endpoint.clone()).build()?;
        let reader = PeriodicReader::builder(exporter).with_interval(Duration::from_secs(5)).build();

        Ok(SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(Resource::builder().with_service_name(self.meter_service_name()).build())
            .build())
    }

    fn init_logs_provider(&self, otel_endpoint: &Url) -> anyhow::Result<SdkLoggerProvider> {
        let exporter =
            opentelemetry_otlp::LogExporter::builder().with_tonic().with_endpoint(otel_endpoint.clone()).build()?;

        Ok(SdkLoggerProvider::builder()
            .with_resource(Resource::builder().with_service_name(self.logger_service_name()).build())
            .with_batch_exporter(exporter)
            .build())
    }
}

#[async_trait::async_trait]
impl Service for TelemetryService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        // SAFETY: `providers.take()` is only safe because `TelemetryService`
        // is non-restartable — see the doc comment on `TelemetryService`.
        // If `Telemetry` is ever re-added to the admin-RPC controllable set,
        // a second `start()` call would capture `providers = None` and the
        // OTEL providers would never be flushed/shut down, silently losing
        // any buffered telemetry.
        let providers = self.providers.take();

        runner.service_loop(move |mut ctx| async move {
            ctx.cancelled().await;

            if let Some(provider) = providers {
                if let Some(logger_provider) = provider.logger_provider {
                    logger_provider.shutdown()?;
                }
                if let Some(meter_provider) = provider.meter_provider {
                    meter_provider.shutdown()?;
                }
                if let Some(tracer_provider) = provider.tracer_provider {
                    tracer_provider.shutdown()?;
                }
            }

            anyhow::Ok(())
        });

        Ok(())
    }
}

impl ServiceId for TelemetryService {
    #[inline(always)]
    fn svc_id(&self) -> mp_utils::service::PowerOfTwo {
        MadaraServiceId::Telemetry.svc_id()
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
