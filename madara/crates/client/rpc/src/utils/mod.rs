mod broadcasted_to_blockifier;
mod execution_error;
mod message_fee;
pub use broadcasted_to_blockifier::tx_api_to_blockifier;
pub use execution_error::{contract_execution_error, contract_execution_error_from_revert};
pub use message_fee::execute_message_fee_estimation;
use std::fmt;

pub fn display_internal_server_error(err: impl fmt::Display) {
    tracing::error!(target: "rpc_errors", "{:#}", err);
}

/// Bounds the number of transactions accepted by an `estimateFee`/`simulateTransactions` request:
/// each transaction can trigger repeated execution during L2 gas limit discovery, so the batch
/// size caps the per-request execution work. `operation` names the request kind in the error
/// message ("estimated" or "simulated").
pub fn check_estimate_batch_size(len: usize, operation: &str) -> Result<(), crate::errors::StarknetRpcApiError> {
    if len > crate::constants::MAX_ESTIMATE_TRANSACTIONS {
        return Err(crate::errors::StarknetRpcApiError::InvalidParams {
            error: format!(
                "Too many transactions: at most {} transactions can be {operation} per request",
                crate::constants::MAX_ESTIMATE_TRANSACTIONS
            )
            .into(),
        });
    }
    Ok(())
}
