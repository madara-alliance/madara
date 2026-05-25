use hyper::StatusCode;
use mc_submit_tx::{RejectedTransactionErrorKind, SubmitTransactionError};
use mc_telemetry::{register_counter_metric_instrument, register_histogram_metric_instrument};
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::{sync::LazyLock, time::Duration};
use url::Url;

pub(crate) mod add_transaction_error_code {
    pub const NONE: &str = "none";
    pub const INTERNAL_SERVER_ERROR: &str = "internal_server_error";
    pub const UNSUPPORTED: &str = "unsupported";
}

pub(crate) mod add_transaction_result {
    pub const SUCCESS: &str = "success";
    pub const REJECTED: &str = "rejected";
    pub const INTERNAL_ERROR: &str = "internal_error";
    pub const UNSUPPORTED: &str = "unsupported";
}

pub(crate) mod add_transaction_tx_type {
    pub const DECLARE: &str = "declare";
    pub const DEPLOY_ACCOUNT: &str = "deploy_account";
    pub const INVOKE: &str = "invoke";
}

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
    add_transaction_total: Counter<u64>,
    add_transaction_duration_seconds: Histogram<f64>,
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
        let add_transaction_total = register_counter_metric_instrument(
            &meter,
            "gateway_client_add_transaction_total".to_string(),
            "Outbound gateway add_transaction outcomes grouped by transaction type and result".to_string(),
            "transaction".to_string(),
        );
        let add_transaction_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "gateway_client_add_transaction_duration_seconds".to_string(),
            "End-to-end duration of outbound gateway add_transaction handling".to_string(),
            "s".to_string(),
        );

        Self {
            requests_total,
            request_duration_seconds,
            retries_total,
            add_transaction_total,
            add_transaction_duration_seconds,
        }
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

    pub fn record_add_transaction(
        &self,
        tx_type: &'static str,
        result: &'static str,
        error_code: &'static str,
        duration: Duration,
    ) {
        self.add_transaction_total.add(
            1,
            &[
                KeyValue::new("tx_type", tx_type),
                KeyValue::new("result", result),
                KeyValue::new("error_code", error_code),
            ],
        );

        self.add_transaction_duration_seconds
            .record(duration.as_secs_f64(), &[KeyValue::new("tx_type", tx_type), KeyValue::new("result", result)]);
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
        "feeder_gateway/get_transaction" => RequestLabels { service: "feeder_gateway", endpoint: "get_transaction" },
        "feeder_gateway/get_transaction_status" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_transaction_status" }
        }
        "feeder_gateway/get_block_hash_by_id" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_block_hash_by_id" }
        }
        "feeder_gateway/get_block_id_by_hash" => {
            RequestLabels { service: "feeder_gateway", endpoint: "get_block_id_by_hash" }
        }
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

pub(crate) fn add_transaction_result_from_submit_error(error: &SubmitTransactionError) -> &'static str {
    match error {
        SubmitTransactionError::Rejected(_) => add_transaction_result::REJECTED,
        SubmitTransactionError::Internal(_) => add_transaction_result::INTERNAL_ERROR,
        SubmitTransactionError::Unsupported => add_transaction_result::UNSUPPORTED,
    }
}

pub(crate) fn add_transaction_error_code_from_submit_error(error: &SubmitTransactionError) -> &'static str {
    match error {
        SubmitTransactionError::Rejected(error) => rejected_transaction_error_code_label(&error.kind),
        SubmitTransactionError::Internal(_) => add_transaction_error_code::INTERNAL_SERVER_ERROR,
        SubmitTransactionError::Unsupported => add_transaction_error_code::UNSUPPORTED,
    }
}

fn rejected_transaction_error_code_label(kind: &RejectedTransactionErrorKind) -> &'static str {
    use RejectedTransactionErrorKind as ErrorKind;

    match kind {
        ErrorKind::EntryPointNotFound => "entry_point_not_found",
        ErrorKind::OutOfRangeContractAddress => "out_of_range_contract_address",
        ErrorKind::TransactionFailed => "transaction_failed",
        ErrorKind::UninitializedContract => "uninitialized_contract",
        ErrorKind::OutOfRangeTransactionHash => "out_of_range_transaction_hash",
        ErrorKind::UnsupportedSelectorForFee => "unsupported_selector_for_fee",
        ErrorKind::InvalidContractDefinition => "invalid_contract_definition",
        ErrorKind::NotPermittedContract => "not_permitted_contract",
        ErrorKind::UndeclaredClass => "undeclared_class",
        ErrorKind::TransactionLimitExceeded => "transaction_limit_exceeded",
        ErrorKind::InvalidTransactionNonce => "invalid_transaction_nonce",
        ErrorKind::ReplacementTransactionUnderpriced => "replacement_transaction_underpriced",
        ErrorKind::FeeBelowMinimum => "fee_below_minimum",
        ErrorKind::OutOfRangeFee => "out_of_range_fee",
        ErrorKind::InvalidTransactionVersion => "invalid_transaction_version",
        ErrorKind::InvalidProgram => "invalid_program",
        ErrorKind::DeprecatedTransaction => "deprecated_transaction",
        ErrorKind::InvalidCompiledClassHash => "invalid_compiled_class_hash",
        ErrorKind::CompilationFailed => "compilation_failed",
        ErrorKind::UnauthorizedEntryPointForInvoke => "unauthorized_entry_point_for_invoke",
        ErrorKind::InvalidContractClass => "invalid_contract_class",
        ErrorKind::ClassAlreadyDeclared => "class_already_declared",
        ErrorKind::InvalidSignature => "invalid_signature",
        ErrorKind::InsufficientAccountBalance => "insufficient_account_balance",
        ErrorKind::InsufficientMaxFee => "insufficient_max_fee",
        ErrorKind::ValidateFailure => "validate_failure",
        ErrorKind::ContractBytecodeSizeTooLarge => "contract_bytecode_size_too_large",
        ErrorKind::ContractClassObjectSizeTooLarge => "contract_class_object_size_too_large",
        ErrorKind::DuplicatedTransaction => "duplicated_transaction",
        ErrorKind::InvalidContractClassVersion => "invalid_contract_class_version",
        ErrorKind::RateLimited => "rate_limited",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_transaction_error_code, add_transaction_error_code_from_submit_error, add_transaction_result,
        add_transaction_result_from_submit_error, request_labels_from_path, request_labels_from_url, RequestLabels,
    };
    use mc_submit_tx::{RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransactionError};
    use std::borrow::Cow;
    use url::Url;

    #[test]
    fn classifies_known_feeder_gateway_path() {
        let labels = request_labels_from_path("/feeder_gateway/get_state_update");
        assert_eq!(labels, RequestLabels { service: "feeder_gateway", endpoint: "get_state_update" });
    }

    #[test]
    fn classifies_pr_1075_feeder_gateway_path() {
        let labels = request_labels_from_path("/feeder_gateway/get_block_hash_by_id");
        assert_eq!(labels, RequestLabels { service: "feeder_gateway", endpoint: "get_block_hash_by_id" });
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

    #[test]
    fn classifies_submit_rejection_labels() {
        let error = SubmitTransactionError::Rejected(RejectedTransactionError {
            kind: RejectedTransactionErrorKind::RateLimited,
            message: Some(Cow::Borrowed("rate limited")),
        });

        assert_eq!(add_transaction_result_from_submit_error(&error), add_transaction_result::REJECTED);
        assert_eq!(add_transaction_error_code_from_submit_error(&error), "rate_limited");
    }

    #[test]
    fn classifies_internal_submit_error_label() {
        let error = SubmitTransactionError::Internal(anyhow::anyhow!("boom"));

        assert_eq!(add_transaction_result_from_submit_error(&error), add_transaction_result::INTERNAL_ERROR);
        assert_eq!(
            add_transaction_error_code_from_submit_error(&error),
            add_transaction_error_code::INTERNAL_SERVER_ERROR
        );
    }
}
