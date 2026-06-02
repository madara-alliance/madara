use crate::error::GatewayError;
use hyper::StatusCode;
use mc_submit_tx::{RejectedTransactionErrorKind, SubmitTransactionError};
use mc_telemetry::{register_counter_metric_instrument, register_histogram_metric_instrument};
use mp_gateway::error::StarknetErrorCode;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::{sync::LazyLock, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RequestLabels {
    pub service: &'static str,
    pub endpoint: &'static str,
}

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
    pub const UNKNOWN: &str = "unknown";
}

pub(crate) struct GatewayServerMetrics {
    requests_total: Counter<u64>,
    request_duration_seconds: Histogram<f64>,
    add_transaction_total: Counter<u64>,
    add_transaction_duration_seconds: Histogram<f64>,
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
        let add_transaction_total = register_counter_metric_instrument(
            &meter,
            "gateway_server_add_transaction_total".to_string(),
            "Gateway add_transaction outcomes grouped by transaction type and result".to_string(),
            "transaction".to_string(),
        );
        let add_transaction_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "gateway_server_add_transaction_duration_seconds".to_string(),
            "End-to-end duration of gateway add_transaction handling".to_string(),
            "s".to_string(),
        );

        Self { requests_total, request_duration_seconds, add_transaction_total, add_transaction_duration_seconds }
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

static METRICS: LazyLock<GatewayServerMetrics> = LazyLock::new(GatewayServerMetrics::register);

pub(crate) fn metrics() -> &'static GatewayServerMetrics {
    &METRICS
}

pub(crate) fn request_labels_from_path(path: &str) -> RequestLabels {
    let normalized = path.trim_matches('/');

    match normalized {
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
        _ if normalized.starts_with("feeder_gateway/") => {
            RequestLabels { service: "feeder_gateway", endpoint: "unknown" }
        }
        _ if normalized.starts_with("gateway/") => RequestLabels { service: "gateway", endpoint: "unknown" },
        _ if normalized.starts_with("madara/") => RequestLabels { service: "madara", endpoint: "unknown" },
        _ => RequestLabels { service: "unknown", endpoint: "unknown" },
    }
}

pub(crate) fn http_method_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        _ => "other",
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

pub(crate) fn add_transaction_result_from_gateway_error(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::StarknetError(_) => add_transaction_result::REJECTED,
        GatewayError::InternalServerError => add_transaction_result::INTERNAL_ERROR,
        GatewayError::Unsupported => add_transaction_result::UNSUPPORTED,
    }
}

pub(crate) fn add_transaction_error_code_from_gateway_error(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::StarknetError(error) => starknet_error_code_label(error.code),
        GatewayError::InternalServerError => add_transaction_error_code::INTERNAL_SERVER_ERROR,
        GatewayError::Unsupported => add_transaction_error_code::UNSUPPORTED,
    }
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

fn starknet_error_code_label(code: StarknetErrorCode) -> &'static str {
    use StarknetErrorCode as ErrorCode;

    match code {
        ErrorCode::BlockNotFound => "block_not_found",
        ErrorCode::NoBlockHeader => "no_block_header",
        ErrorCode::EntryPointNotFound => "entry_point_not_found",
        ErrorCode::OutOfRangeContractAddress => "out_of_range_contract_address",
        ErrorCode::SchemaValidationError => "schema_validation_error",
        ErrorCode::TransactionFailed => "transaction_failed",
        ErrorCode::UninitializedContract => "uninitialized_contract",
        ErrorCode::OutOfRangeBlockHash => "out_of_range_block_hash",
        ErrorCode::OutOfRangeTransactionHash => "out_of_range_transaction_hash",
        ErrorCode::MalformedRequest => "malformed_request",
        ErrorCode::UnsupportedSelectorForFee => "unsupported_selector_for_fee",
        ErrorCode::InvalidContractDefinition => "invalid_contract_definition",
        ErrorCode::NotPermittedContract => "not_permitted_contract",
        ErrorCode::UndeclaredClass => "undeclared_class",
        ErrorCode::TransactionLimitExceeded => "transaction_limit_exceeded",
        ErrorCode::InvalidTransactionNonce => "invalid_transaction_nonce",
        ErrorCode::ReplacementTransactionUnderpriced => "replacement_transaction_underpriced",
        ErrorCode::FeeBelowMinimum => "fee_below_minimum",
        ErrorCode::OutOfRangeFee => "out_of_range_fee",
        ErrorCode::InvalidTransactionVersion => "invalid_transaction_version",
        ErrorCode::InvalidProgram => "invalid_program",
        ErrorCode::DeprecatedTransaction => "deprecated_transaction",
        ErrorCode::InvalidCompiledClassHash => "invalid_compiled_class_hash",
        ErrorCode::CompilationFailed => "compilation_failed",
        ErrorCode::UnauthorizedEntryPointForInvoke => "unauthorized_entry_point_for_invoke",
        ErrorCode::InvalidContractClass => "invalid_contract_class",
        ErrorCode::ClassAlreadyDeclared => "class_already_declared",
        ErrorCode::InvalidSignature => "invalid_signature",
        ErrorCode::NoSignatureForPendingBlock => "no_signature_for_pending_block",
        ErrorCode::InsufficientAccountBalance => "insufficient_account_balance",
        ErrorCode::InsufficientMaxFee => "insufficient_max_fee",
        ErrorCode::ValidateFailure => "validate_failure",
        ErrorCode::ContractBytecodeSizeTooLarge => "contract_bytecode_size_too_large",
        ErrorCode::ContractClassObjectSizeTooLarge => "contract_class_object_size_too_large",
        ErrorCode::DuplicatedTransaction => "duplicated_transaction",
        ErrorCode::InvalidContractClassVersion => "invalid_contract_class_version",
        ErrorCode::RateLimited => "rate_limited",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_transaction_error_code, add_transaction_error_code_from_gateway_error,
        add_transaction_error_code_from_submit_error, add_transaction_result,
        add_transaction_result_from_gateway_error, add_transaction_result_from_submit_error, http_method_label,
        request_labels_from_path, status_class_label, RequestLabels,
    };
    use crate::error::GatewayError;
    use hyper::StatusCode;
    use mc_submit_tx::{RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransactionError};
    use mp_gateway::error::{StarknetError, StarknetErrorCode};
    use std::borrow::Cow;

    #[test]
    fn classifies_known_route() {
        let labels = request_labels_from_path("feeder_gateway/get_signature");
        assert_eq!(labels, RequestLabels { service: "feeder_gateway", endpoint: "get_signature" });
    }

    #[test]
    fn classifies_pr_1075_route() {
        let labels = request_labels_from_path("feeder_gateway/get_transaction_status");
        assert_eq!(labels, RequestLabels { service: "feeder_gateway", endpoint: "get_transaction_status" });
    }

    #[test]
    fn collapses_unknown_route() {
        let labels = request_labels_from_path("unknown/path");
        assert_eq!(labels, RequestLabels { service: "unknown", endpoint: "unknown" });
    }

    #[test]
    fn classifies_slash_prefixed_unknown_gateway_route() {
        let labels = request_labels_from_path("/gateway/something_else");
        assert_eq!(labels, RequestLabels { service: "gateway", endpoint: "unknown" });
    }

    #[test]
    fn maps_status_class() {
        assert_eq!(status_class_label(StatusCode::BAD_REQUEST), "4xx");
        assert_eq!(status_class_label(StatusCode::INTERNAL_SERVER_ERROR), "5xx");
    }

    #[test]
    fn bounds_http_method_labels() {
        assert_eq!(http_method_label("GET"), "GET");
        assert_eq!(http_method_label("POST"), "POST");
        assert_eq!(http_method_label("PATCH"), "other");
        assert_eq!(http_method_label("CUSTOM_METHOD"), "other");
    }

    #[test]
    fn classifies_submit_rejection_labels() {
        let error = SubmitTransactionError::Rejected(RejectedTransactionError {
            kind: RejectedTransactionErrorKind::DuplicatedTransaction,
            message: Some(Cow::Borrowed("duplicate")),
        });

        assert_eq!(add_transaction_result_from_submit_error(&error), add_transaction_result::REJECTED);
        assert_eq!(add_transaction_error_code_from_submit_error(&error), "duplicated_transaction");
    }

    #[test]
    fn classifies_gateway_error_labels() {
        let error =
            GatewayError::StarknetError(StarknetError::new(StarknetErrorCode::MalformedRequest, "bad body".into()));

        assert_eq!(add_transaction_result_from_gateway_error(&error), add_transaction_result::REJECTED);
        assert_eq!(add_transaction_error_code_from_gateway_error(&error), "malformed_request");
    }

    #[test]
    fn classifies_internal_gateway_error_label() {
        let error = GatewayError::InternalServerError;

        assert_eq!(add_transaction_result_from_gateway_error(&error), add_transaction_result::INTERNAL_ERROR);
        assert_eq!(
            add_transaction_error_code_from_gateway_error(&error),
            add_transaction_error_code::INTERNAL_SERVER_ERROR
        );
    }
}
