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
) -> Gauge<f64> {
    crate_meter.f64_gauge(instrument_name).with_description(desc).with_unit(unit).build()
}

pub fn register_counter_metric_instrument(
    crate_meter: &Meter,
    instrument_name: String,
    desc: String,
    unit: String,
) -> Counter<f64> {
    crate_meter.f64_counter(instrument_name).with_description(desc).with_unit(unit).build()
}
