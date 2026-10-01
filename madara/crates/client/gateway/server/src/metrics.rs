use mc_telemetry::register_counter_metric_instrument;
use opentelemetry::{global, metrics::Counter, InstrumentationScope, KeyValue};

#[derive(Clone, Debug)]
pub(crate) struct GatewayMetrics {
    uncompressed_response_body_bytes: Counter<u64>,
    transmitted_response_body_bytes: Counter<u64>,
}

impl GatewayMetrics {
    pub(crate) fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.gateway.opentelemetry")
                .with_attributes([KeyValue::new("crate", "gateway")])
                .build(),
        );

        let uncompressed_response_body_bytes = register_counter_metric_instrument(
            &meter,
            "feeder_gateway_response_body_uncompressed_bytes".to_string(),
            "Logical feeder response-body bytes before gzip compression".to_string(),
            "bytes".to_string(),
        );
        let transmitted_response_body_bytes = register_counter_metric_instrument(
            &meter,
            "feeder_gateway_response_body_transmitted_bytes".to_string(),
            "Feeder response-body bytes transmitted after optional gzip compression".to_string(),
            "bytes".to_string(),
        );

        Self { uncompressed_response_body_bytes, transmitted_response_body_bytes }
    }

    pub(crate) fn record_response(
        &self,
        route: &'static str,
        encoding: &'static str,
        uncompressed_bytes: u64,
        transmitted_bytes: u64,
    ) {
        let attributes = [KeyValue::new("route", route), KeyValue::new("encoding", encoding)];
        self.uncompressed_response_body_bytes.add(uncompressed_bytes, &attributes);
        self.transmitted_response_body_bytes.add(transmitted_bytes, &attributes);
    }
}
