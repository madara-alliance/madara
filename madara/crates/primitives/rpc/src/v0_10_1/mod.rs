//! v0.10.1 of the Starknet JSON-RPC API.
//!
//! Changes from v0.10.0:
//! 1. Address array filter in `starknet_getEvents` - supports single or multiple addresses
//! 2. RETURN_INITIAL_READS simulation flag for `simulateTransactions` and `traceBlockTransactions`
//! 3. proof_facts field in INVOKE_TXN_V3 for transactions submitted with proof facts
//! 4. response_flags parameter for transaction-returning methods (getTransactionByHash, getBlockWithTxs, etc.)
//! 5. proof field in broadcasted invoke v3 transactions
//! 6. INCLUDE_PROOF_FACTS tag for WebSocket subscriptions

mod starknet_api_openrpc;
mod starknet_trace_api_openrpc;
mod starknet_ws_api;

pub use self::starknet_api_openrpc::*;
pub use self::starknet_trace_api_openrpc::*;
pub use self::starknet_ws_api::*;

// BlockId is unchanged from v0.10.0, so we reuse the same type
pub use crate::v0_10_0::BlockId;

#[cfg(test)]
mod tests;
