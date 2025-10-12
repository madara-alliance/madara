use super::starknet_api_openrpc::*;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub use crate::v0_8_1::{
    CallType, DeclareTransactionTrace, DeployAccountTransactionTrace, EntryPointType, InvokeTransactionTrace,
    OrderedEvent, OrderedMessage, RevertedInvocation, RevertibleFunctionInvocation, SimulationFlag,
};

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum TransactionTrace {
    /// the execution trace of an invoke transaction
    #[serde(rename = "INVOKE")]
    Invoke(InvokeTransactionTrace),
    /// the execution trace of a declare transaction
    #[serde(rename = "DECLARE")]
    Declare(DeclareTransactionTrace),
    /// the execution trace of a deploy account transaction
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(DeployAccountTransactionTrace),
    /// the execution trace of an L1 handler transaction
    #[serde(rename = "L1_HANDLER")]
    L1Handler(L1HandlerTransactionTrace),
}

/// the execution trace of an L1 handler transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct L1HandlerTransactionTrace {
    /// the trace of the L1 handler call
    pub execution_resources: ExecutionResources,
    /// the trace of the __execute__ call or constructor call, depending on the transaction type (none for declare transactions)
    pub function_invocation: RevertibleFunctionInvocation,
    /// the state diffs induced by the transaction
    #[serde(default)]
    pub state_diff: Option<StateDiff>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SimulateTransactionsResult {
    pub fee_estimation: FeeEstimate,
    pub transaction_trace: TransactionTrace,
}

/// A single pair of transaction hash and corresponding trace
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TraceBlockTransactionsResult {
    pub trace_root: TransactionTrace,
    pub transaction_hash: Felt,
}

/// Trace of a single transaction returned by `starknet_traceTransaction`
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TraceTransactionResult {
    #[serde(flatten)]
    pub trace: TransactionTrace,
}
