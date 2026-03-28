use super::starknet_api_openrpc::{ExecutionResources, FeeEstimate, StateDiff};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

// Re-export unchanged trace types from v0.8.1/v0.9
pub use crate::v0_8_1::{
    CallType, EntryPointType, FunctionInvocation, InnerCallExecutionResources, OrderedEvent, OrderedMessage,
    RevertedInvocation, RevertibleFunctionInvocation, SimulationFlag,
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

// Conversions from v0.9 trace types to v0.10

impl From<crate::v0_7_1::StateDiff> for StateDiff {
    fn from(sd: crate::v0_7_1::StateDiff) -> Self {
        StateDiff {
            declared_classes: sd.declared_classes,
            deployed_contracts: sd.deployed_contracts,
            deprecated_declared_classes: sd.deprecated_declared_classes,
            nonces: sd.nonces,
            replaced_classes: sd.replaced_classes,
            storage_diffs: sd.storage_diffs,
            // Migrated compiled classes are computed only during block finalization
            // (BlockExecutionSummary), not during per-transaction execution. The trace
            // code path uses per-tx TransactionalState which doesn't expose migration info.
            migrated_compiled_classes: vec![],
        }
    }
}

impl From<crate::v0_9_0::TransactionTrace> for TransactionTrace {
    fn from(trace: crate::v0_9_0::TransactionTrace) -> Self {
        match trace {
            crate::v0_9_0::TransactionTrace::Invoke(t) => TransactionTrace::Invoke(t.into()),
            crate::v0_9_0::TransactionTrace::Declare(t) => TransactionTrace::Declare(t.into()),
            crate::v0_9_0::TransactionTrace::DeployAccount(t) => TransactionTrace::DeployAccount(t.into()),
            crate::v0_9_0::TransactionTrace::L1Handler(t) => TransactionTrace::L1Handler(t.into()),
        }
    }
}

impl From<crate::v0_8_1::InvokeTransactionTrace> for InvokeTransactionTrace {
    fn from(t: crate::v0_8_1::InvokeTransactionTrace) -> Self {
        InvokeTransactionTrace {
            execute_invocation: t.execute_invocation,
            execution_resources: t.execution_resources.into(),
            fee_transfer_invocation: t.fee_transfer_invocation,
            state_diff: t.state_diff.map(Into::into),
            validate_invocation: t.validate_invocation,
        }
    }
}

impl From<crate::v0_8_1::DeclareTransactionTrace> for DeclareTransactionTrace {
    fn from(t: crate::v0_8_1::DeclareTransactionTrace) -> Self {
        DeclareTransactionTrace {
            execution_resources: t.execution_resources.into(),
            fee_transfer_invocation: t.fee_transfer_invocation,
            state_diff: t.state_diff.map(Into::into),
            validate_invocation: t.validate_invocation,
        }
    }
}

impl From<crate::v0_8_1::DeployAccountTransactionTrace> for DeployAccountTransactionTrace {
    fn from(t: crate::v0_8_1::DeployAccountTransactionTrace) -> Self {
        DeployAccountTransactionTrace {
            constructor_invocation: t.constructor_invocation,
            execution_resources: t.execution_resources.into(),
            fee_transfer_invocation: t.fee_transfer_invocation,
            state_diff: t.state_diff.map(Into::into),
            validate_invocation: t.validate_invocation,
        }
    }
}

impl From<crate::v0_9_0::L1HandlerTransactionTrace> for L1HandlerTransactionTrace {
    fn from(t: crate::v0_9_0::L1HandlerTransactionTrace) -> Self {
        L1HandlerTransactionTrace {
            execution_resources: t.execution_resources.into(),
            function_invocation: t.function_invocation,
            state_diff: t.state_diff.map(Into::into),
        }
    }
}

impl From<crate::v0_9_0::TraceTransactionResult> for TraceTransactionResult {
    fn from(t: crate::v0_9_0::TraceTransactionResult) -> Self {
        TraceTransactionResult { trace: t.trace.into() }
    }
}

impl From<crate::v0_9_0::TraceBlockTransactionsResult> for TraceBlockTransactionsResult {
    fn from(t: crate::v0_9_0::TraceBlockTransactionsResult) -> Self {
        TraceBlockTransactionsResult { trace_root: t.trace_root.into(), transaction_hash: t.transaction_hash }
    }
}

impl From<crate::v0_9_0::SimulateTransactionsResult> for SimulateTransactionsResult {
    fn from(t: crate::v0_9_0::SimulateTransactionsResult) -> Self {
        SimulateTransactionsResult { fee_estimation: t.fee_estimation, transaction_trace: t.transaction_trace.into() }
    }
}
