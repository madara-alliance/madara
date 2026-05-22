use hyper::StatusCode;
use mc_telemetry::{register_counter_metric_instrument, register_histogram_metric_instrument};
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::{sync::LazyLock, time::Duration};
use url::Url;

pub(crate) mod error_kind {
    pub const NONE: &str = "none";
    pub const REQUEST_BUILD: &str = "request_build";
    pub const REQUEST_SERIALIZE: &str = "request_serialize";
    pub const TRANSPORT: &str = "transport";
    pub const STARKNET: &str = "starknet";
    pub const INVALID_STARKNET: &str = "invalid_starknet";
    pub const RESPONSE_DESERIALIZE: &str = "response_deserialize";
}

pub(crate) mod request_result {
    pub const SUCCESS: &str = "success";
    pub const FAILURE: &str = "failure";
}

pub(crate) mod retry_reason {
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const TRANSPORT: &str = "transport";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RequestLabels {
    pub service: &'static str,
    pub endpoint: &'static str,
}

pub(crate) struct GatewayClientMetrics {
    requests_total: Counter<u64>,
    request_duration_seconds: Histogram<f64>,
    retries_total: Counter<u64>,
}

impl GatewayClientMetrics {
    fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.gateway_client.opentelemetry")
                .with_attributes([KeyValue::new("crate", "gateway_client")])
                .build(),
        );

        let requests_total = register_counter_metric_instrument(
            &meter,
            "gateway_client_requests_total".to_string(),
            "Total outbound gateway and feeder-gateway requests".to_string(),
            "request".to_string(),
        );
        let request_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "gateway_client_request_duration_seconds".to_string(),
            "End-to-end duration of outbound gateway and feeder-gateway requests".to_string(),
            "s".to_string(),
        );
        let retries_total = register_counter_metric_instrument(
            &meter,
            "gateway_client_retries_total".to_string(),
            "Retry attempts for outbound gateway and feeder-gateway requests".to_string(),
            "retry".to_string(),
        );

        Self { requests_total, request_duration_seconds, retries_total }
    }

    pub fn record_request(
        &self,
        labels: RequestLabels,
        http_method: &str,
        result: &'static str,
        error_kind: &'static str,
        status_code: Option<StatusCode>,
        duration: Duration,
    ) {
        let status_code = status_code_label(status_code);

        self.requests_total.add(
            1,
            &[
                KeyValue::new("service", labels.service),
                KeyValue::new("endpoint", labels.endpoint),
                KeyValue::new("http_method", http_method.to_string()),
                KeyValue::new("result", result),
                KeyValue::new("error_kind", error_kind),
                KeyValue::new("status_code", status_code),
            ],
        );

        self.request_duration_seconds.record(
            duration.as_secs_f64(),
            &[
                KeyValue::new("service", labels.service),
                KeyValue::new("endpoint", labels.endpoint),
                KeyValue::new("http_method", http_method.to_string()),
                KeyValue::new("result", result),
                KeyValue::new("error_kind", error_kind),
            ],
        );
    }

    pub fn record_retry(&self, labels: RequestLabels, http_method: &str, reason: &'static str) {
        self.retries_total.add(
            1,
            &[
                KeyValue::new("service", labels.service),
                KeyValue::new("endpoint", labels.endpoint),
                KeyValue::new("http_method", http_method.to_string()),
                KeyValue::new("reason", reason),
            ],
        );
    }
}

static METRICS: LazyLock<GatewayClientMetrics> = LazyLock::new(GatewayClientMetrics::register);

pub(crate) fn metrics() -> &'static GatewayClientMetrics {
    &METRICS
}

pub(crate) fn request_labels_from_url(url: &Url) -> RequestLabels {
    request_labels_from_path(url.path())
}

pub(crate) fn request_labels_from_path(path: &str) -> RequestLabels {
    let normalized = path.trim_matches('/');

    match normalized {
        "feeder_gateway/get_block" => RequestLabels { service: "feeder_gateway", endpoint: "get_block" },
        "feeder_gateway/get_preconfirmed_block" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_preconfirmed_block" }
        }
        "feeder_gateway/get_state_update" => RequestLabels { service: "feeder_gateway", endpoint: "get_state_update" },
        "feeder_gateway/get_block_bouncer_weights" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_block_bouncer_weights" }
        }
        "feeder_gateway/get_signature" => RequestLabels { service: "feeder_gateway", endpoint: "get_signature" },
        "feeder_gateway/get_class_by_hash" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_class_by_hash" }
        }
        "gateway/add_transaction" => RequestLabels { service: "gateway", endpoint: "add_transaction" },
        "madara/trusted_add_validated_transaction" => {
            RequestLabels { service: "madara", endpoint: "trusted_add_validated_transaction" }
        }
        _ if normalized.starts_with("feeder_gateway/") => {
            RequestLabels { service: "feeder_gateway", endpoint: "unknown" }
        }
        _ if normalized.starts_with("gateway/") => RequestLabels { service: "gateway", endpoint: "unknown" },
        _ if normalized.starts_with("madara/") => RequestLabels { service: "madara", endpoint: "unknown" },
        _ => RequestLabels { service: "unknown", endpoint: "unknown" },
    }
}

fn status_code_label(status_code: Option<StatusCode>) -> String {
    status_code.map(|status| status.as_u16().to_string()).unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
mod tests {
    use super::{request_labels_from_path, request_labels_from_url, RequestLabels};
    use url::Url;

    #[test]
    fn classifies_known_feeder_gateway_path() {
        let labels = request_labels_from_path("/feeder_gateway/get_state_update");
        assert_eq!(labels, RequestLabels { service: "feeder_gateway", endpoint: "get_state_update" });
    }

    #[test]
    fn collapses_unknown_gateway_endpoint() {
        let labels = request_labels_from_path("/gateway/something_else");
        assert_eq!(labels, RequestLabels { service: "gateway", endpoint: "unknown" });
    }

    #[test]
    fn classifies_url_path() {
        let url = Url::parse("https://example.com/madara/trusted_add_validated_transaction").unwrap();
        let labels = request_labels_from_url(&url);
        assert_eq!(labels, RequestLabels { service: "madara", endpoint: "trusted_add_validated_transaction" });
    }
}
