use hyper::StatusCode;
use mc_telemetry::{register_counter_metric_instrument, register_histogram_metric_instrument};
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::{sync::LazyLock, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RequestLabels {
    pub service: &'static str,
    pub endpoint: &'static str,
}

pub(crate) struct GatewayServerMetrics {
    requests_total: Counter<u64>,
    request_duration_seconds: Histogram<f64>,
}

impl GatewayServerMetrics {
    fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.gateway_server.opentelemetry")
                .with_attributes([KeyValue::new("crate", "gateway_server")])
                .build(),
        );

        let requests_total = register_counter_metric_instrument(
            &meter,
            "gateway_server_requests_total".to_string(),
            "Total inbound gateway and feeder-gateway requests".to_string(),
            "request".to_string(),
        );
        let request_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "gateway_server_request_duration_seconds".to_string(),
            "End-to-end duration of inbound gateway and feeder-gateway requests".to_string(),
            "s".to_string(),
        );

        Self { requests_total, request_duration_seconds }
    }

    pub fn record_request(
        &self,
        labels: RequestLabels,
        http_method: &str,
        status_code: StatusCode,
        duration: Duration,
    ) {
        let status_class = status_class_label(status_code);

        self.requests_total.add(
            1,
            &[
                KeyValue::new("service", labels.service),
                KeyValue::new("endpoint", labels.endpoint),
                KeyValue::new("http_method", http_method.to_string()),
                KeyValue::new("status_code", status_code.as_u16().to_string()),
                KeyValue::new("status_class", status_class),
            ],
        );

        self.request_duration_seconds.record(
            duration.as_secs_f64(),
            &[
                KeyValue::new("service", labels.service),
                KeyValue::new("endpoint", labels.endpoint),
                KeyValue::new("http_method", http_method.to_string()),
                KeyValue::new("status_class", status_class),
            ],
        );
    }
}

static METRICS: LazyLock<GatewayServerMetrics> = LazyLock::new(GatewayServerMetrics::register);

pub(crate) fn metrics() -> &'static GatewayServerMetrics {
    &METRICS
}

pub(crate) fn request_labels_from_path(path: &str) -> RequestLabels {
    match path.trim_matches('/') {
        "health" => RequestLabels { service: "health", endpoint: "health" },
        "gateway/add_transaction" => RequestLabels { service: "gateway", endpoint: "add_transaction" },
        "madara/trusted_add_validated_transaction" => {
            RequestLabels { service: "madara", endpoint: "trusted_add_validated_transaction" }
        }
        "feeder_gateway/get_preconfirmed_block" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_preconfirmed_block" }
        }
        "feeder_gateway/get_block" => RequestLabels { service: "feeder_gateway", endpoint: "get_block" },
        "feeder_gateway/get_signature" => RequestLabels { service: "feeder_gateway", endpoint: "get_signature" },
        "feeder_gateway/get_state_update" => RequestLabels { service: "feeder_gateway", endpoint: "get_state_update" },
        "feeder_gateway/get_block_traces" => RequestLabels { service: "feeder_gateway", endpoint: "get_block_traces" },
        "feeder_gateway/get_class_by_hash" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_class_by_hash" }
        }
        "feeder_gateway/get_compiled_class_by_class_hash" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_compiled_class_by_class_hash" }
        }
        "feeder_gateway/get_contract_addresses" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_contract_addresses" }
        }
        "feeder_gateway/get_public_key" => RequestLabels { service: "feeder_gateway", endpoint: "get_public_key" },
        "feeder_gateway/get_block_bouncer_weights" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_block_bouncer_weights" }
        }
        _ if path.starts_with("feeder_gateway/") => RequestLabels { service: "feeder_gateway", endpoint: "unknown" },
        _ if path.starts_with("gateway/") => RequestLabels { service: "gateway", endpoint: "unknown" },
        _ if path.starts_with("madara/") => RequestLabels { service: "madara", endpoint: "unknown" },
        _ => RequestLabels { service: "unknown", endpoint: "unknown" },
    }
}

pub(crate) fn status_class_label(status_code: StatusCode) -> &'static str {
    if status_code.is_informational() {
        "1xx"
    } else if status_code.is_success() {
        "2xx"
    } else if status_code.is_redirection() {
        "3xx"
    } else if status_code.is_client_error() {
        "4xx"
    } else {
        "5xx"
    }
}

#[cfg(test)]
mod tests {
    use super::{request_labels_from_path, status_class_label, RequestLabels};
    use hyper::StatusCode;

    #[test]
    fn classifies_known_route() {
        let labels = request_labels_from_path("feeder_gateway/get_signature");
        assert_eq!(labels, RequestLabels { service: "feeder_gateway", endpoint: "get_signature" });
    }

    #[test]
    fn collapses_unknown_route() {
        let labels = request_labels_from_path("unknown/path");
        assert_eq!(labels, RequestLabels { service: "unknown", endpoint: "unknown" });
    }

    #[test]
    fn maps_status_class() {
        assert_eq!(status_class_label(StatusCode::BAD_REQUEST), "4xx");
        assert_eq!(status_class_label(StatusCode::INTERNAL_SERVER_ERROR), "5xx");
    }
}
