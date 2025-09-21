use super::{
    BlockId, BroadcastedTxn, ComputationResources, EventContent, ExecutionResources, FeeEstimate, FunctionCall,
    MsgToL1, StateDiff, TxnHash,
};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum CallType {
    #[serde(rename = "CALL")]
    Regular,
    #[serde(rename = "DELEGATE")]
    Delegate,
    #[serde(rename = "LIBRARY_CALL")]
    LibraryCall,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum EntryPointType {
    #[serde(rename = "CONSTRUCTOR")]
    Constructor,
    #[serde(rename = "EXTERNAL")]
    External,
    #[serde(rename = "L1_HANDLER")]
    L1Handler,
}

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
    /// Resources consumed by the internal call. This is named execution_resources for legacy reasons
    pub execution_resources: ComputationResources,
    /// The messages sent by this invocation to L1
    pub messages: Vec<OrderedMessage>,
    /// The value returned from the function invocation
    pub result: Vec<Felt>,
}

pub type NestedCall = FunctionInvocation;

/// an event alongside its order within the transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct OrderedEvent {
    /// the order of the event within the transaction
    pub order: u64,
    #[serde(flatten)]
    pub event: EventContent,
}

/// a message alongside its order within the transaction
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct OrderedMessage {
    /// the order of the message within the transaction
    pub order: u64,
    #[serde(flatten)]
    pub msg_to_l_1: MsgToL1,
}

/// Flags that indicate how to simulate a given transaction. By default, the sequencer behavior is replicated locally (enough funds are expected to be in the account, and fee will be deducted from the balance before the simulation of the next transaction). To skip the fee charge, use the SKIP_FEE_CHARGE flag.
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum SimulationFlag {
    #[serde(rename = "SKIP_FEE_CHARGE")]
    SkipFeeCharge,
    #[serde(rename = "SKIP_VALIDATE")]
    SkipValidate,
}

/// the execution trace of an invoke transaction
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum TransactionTrace {
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
    pub execute_invocation: ExecuteInvocation,
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

/// the trace of the __execute__ call or constructor call, depending on the transaction type (none for declare transactions)
#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ExecuteInvocation {
    FunctionInvocation(FunctionInvocation),
    Anon(RevertedInvocation),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RevertedInvocation {
    /// the revert reason for the failed execution
    pub revert_reason: String,
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

/// Parameters of the `starknet_traceTransaction` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceTransactionParams {
    /// The hash of the transaction to trace
    pub transaction_hash: TxnHash,
}

impl Serialize for TraceTransactionParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("transaction_hash", &self.transaction_hash)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for TraceTransactionParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = TraceTransactionParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_traceTransaction`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let transaction_hash: TxnHash =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 1 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &"expected 1 parameters"));
                }

                Ok(TraceTransactionParams { transaction_hash })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    transaction_hash: TxnHash,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(TraceTransactionParams { transaction_hash: helper.transaction_hash })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_simulateTransactions` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulateTransactionsParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag, for the block referencing the state or call the transaction on.
    pub block_id: BlockId,
    /// The transactions to simulate
    pub transactions: Vec<BroadcastedTxn>,
    /// describes what parts of the transaction should be executed
    pub simulation_flags: Vec<SimulationFlag>,
}

impl Serialize for SimulateTransactionsParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.serialize_entry("transactions", &self.transactions)?;
        map.serialize_entry("simulation_flags", &self.simulation_flags)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for SimulateTransactionsParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SimulateTransactionsParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_simulateTransactions`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 3 parameters"))?;
                let transactions: Vec<BroadcastedTxn> =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 3 parameters"))?;
                let simulation_flags: Vec<SimulationFlag> =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(3, &"expected 3 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(4, &"expected 3 parameters"));
                }

                Ok(SimulateTransactionsParams { block_id, transactions, simulation_flags })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                    transactions: Vec<BroadcastedTxn>,
                    simulation_flags: Vec<SimulationFlag>,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(SimulateTransactionsParams {
                    block_id: helper.block_id,
                    transactions: helper.transactions,
                    simulation_flags: helper.simulation_flags,
                })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_traceBlockTransactions` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceBlockTransactionsParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for TraceBlockTransactionsParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for TraceBlockTransactionsParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = TraceBlockTransactionsParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_traceBlockTransactions`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 1 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &"expected 1 parameters"));
                }

                Ok(TraceBlockTransactionsParams { block_id })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(TraceBlockTransactionsParams { block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}
