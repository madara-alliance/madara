mod broadcasted_to_blockifier;
mod message_fee;
pub use broadcasted_to_blockifier::tx_api_to_blockifier;
pub use message_fee::execute_message_fee_estimation;
use std::fmt;

pub fn display_internal_server_error(err: impl fmt::Display) {
    tracing::error!(target: "rpc_errors", "{:#}", err);
}
