mod broadcasted_to_blockifier;
pub use broadcasted_to_blockifier::tx_api_to_blockifier;
use std::fmt;

pub fn display_internal_server_error(err: impl fmt::Display) {
    tracing::error!(target: "rpc_errors", "{:#}", err);
}
