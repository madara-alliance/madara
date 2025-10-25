use super::starknet_api_openrpc::*;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub use crate::v0_7_1::{CallType, EntryPointType, OrderedEvent, OrderedMessage, RevertedInvocation, SimulationFlag};

pub type NestedCall = FunctionInvocation;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FunctionInvocation {
    #[serde(flatten)]
    pub function_call: FunctionCall,
    pub call_type: CallType,
    /// The address of the invoking contract. 0 for the root invocation
    pub caller_address: Felt,
    /// The calls made by this invocation
    pub calls: Vec<NestedCall>,
    /// The hash of the class being called
    pub class_hash: Felt,
    pub entry_point_type: EntryPointType,
    /// The events emitted in this invocation
    pub events: Vec<OrderedEvent>,
    /// Resources consumed by the call tree rooted at this given call (including the root)
    pub execution_resources: InnerCallExecutionResources,
    /// The messages sent by this invocation to L1
    pub messages: Vec<OrderedMessage>,
    /// The value returned from the function invocation
    pub result: Vec<Felt>,
    /// true if this inner call panicked
    pub is_reverted: bool,
}

/// the trace of the __execute__ call or constructor call, depending on the transaction type (none for declare transactions)
#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum RevertibleFunctionInvocation {
    FunctionInvocation(FunctionInvocation),
    Anon(RevertedInvocation),
}

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

/// the execution trace of an invoke transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct InvokeTransactionTrace {
    /// the trace of the __execute__ call or constructor call, depending on the transaction type (none for declare transactions)
    pub execute_invocation: RevertibleFunctionInvocation,
    /// the resources consumed by the transaction, includes both computation and data
    pub execution_resources: ExecutionResources,
    #[serde(default)]
    pub fee_transfer_invocation: Option<FunctionInvocation>,
    /// the state diffs induced by the transaction
    #[serde(default)]
    pub state_diff: Option<StateDiff>,
    #[serde(default)]
    pub validate_invocation: Option<FunctionInvocation>,
}

/// the execution trace of a declare transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeclareTransactionTrace {
    /// the resources consumed by the transaction, includes both computation and data
    pub execution_resources: ExecutionResources,
    #[serde(default)]
    pub fee_transfer_invocation: Option<FunctionInvocation>,
    /// the state diffs induced by the transaction
    #[serde(default)]
    pub state_diff: Option<StateDiff>,
    #[serde(default)]
    pub validate_invocation: Option<FunctionInvocation>,
}

/// the execution trace of a deploy account transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployAccountTransactionTrace {
    /// the trace of the __execute__ call or constructor call, depending on the transaction type (none for declare transactions)
    pub constructor_invocation: FunctionInvocation,
    /// the resources consumed by the transaction, includes both computation and data
    pub execution_resources: ExecutionResources,
    #[serde(default)]
    pub fee_transfer_invocation: Option<FunctionInvocation>,
    /// the state diffs induced by the transaction
    #[serde(default)]
    pub state_diff: Option<StateDiff>,
    #[serde(default)]
    pub validate_invocation: Option<FunctionInvocation>,
}

/// the execution trace of an L1 handler transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct L1HandlerTransactionTrace {
    /// the resources consumed by the transaction, includes both computation and data
    pub execution_resources: ExecutionResources,
    /// the trace of the __execute__ call or constructor call, depending on the transaction type (none for declare transactions)
    pub function_invocation: FunctionInvocation,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Default)]
pub struct InnerCallExecutionResources {
    pub l1_gas: u128,
    pub l2_gas: u128,
}
