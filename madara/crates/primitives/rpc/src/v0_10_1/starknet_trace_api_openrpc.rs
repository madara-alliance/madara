// Re-export base trace types from v0.10.0 (which re-exports from v0.9.0)
pub use crate::v0_10_0::{
    CallType, DeclareTransactionTrace, DeployAccountTransactionTrace, EntryPointType, InvokeTransactionTrace,
    L1HandlerTransactionTrace, OrderedEvent, OrderedMessage, RevertedInvocation, RevertibleFunctionInvocation,
    TraceTransactionResult, TransactionTrace,
};

use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

/// Simulation flag (MODIFIED in v0.10.1)
///
/// Added RETURN_INITIAL_READS flag for collecting initial state reads during simulation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationFlag {
    /// Skip the fee charge during simulation
    #[serde(rename = "SKIP_FEE_CHARGE")]
    SkipFeeCharge,
    /// Skip transaction validation during simulation
    #[serde(rename = "SKIP_VALIDATE")]
    SkipValidate,
    /// Return the initial reads (storage, nonces, class_hashes) during simulation (NEW in v0.10.1)
    #[serde(rename = "RETURN_INITIAL_READS")]
    ReturnInitialReads,
}

/// Trace flag for traceBlockTransactions (NEW in v0.10.1)
///
/// Controls what additional data is returned when tracing block transactions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceFlag {
    /// Return the initial reads during tracing
    #[serde(rename = "RETURN_INITIAL_READS")]
    ReturnInitialReads,
}

/// Initial storage read item (NEW in v0.10.1)
///
/// Represents a single storage read that occurred before any write to that cell.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialStorageRead {
    /// The contract address where the storage was read
    pub contract_address: Felt,
    /// The storage key that was read
    pub key: Felt,
    /// The value that was read from storage
    pub value: Felt,
}

/// Initial nonce read item (NEW in v0.10.1)
///
/// Represents a nonce read that occurred before any modification.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialNonceRead {
    /// The contract address whose nonce was read
    pub contract_address: Felt,
    /// The nonce value that was read
    pub nonce: Felt,
}

/// Initial class hash read item (NEW in v0.10.1)
///
/// Represents a class hash lookup that occurred during execution.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialClassHashRead {
    /// The contract address whose class hash was read
    pub contract_address: Felt,
    /// The class hash that was read
    pub class_hash: Felt,
}

/// Initial declared contract item (NEW in v0.10.1)
///
/// Represents a check whether a class was declared during execution.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialDeclaredContract {
    /// The class hash that was checked
    pub class_hash: Felt,
    /// Whether the class was declared at the time of the read
    pub is_declared: bool,
}

/// Complete initial reads structure (NEW in v0.10.1)
///
/// Contains all the initial state reads that occurred during transaction execution,
/// providing a complete witness sufficient to reconstruct cached state for re-execution.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialReads {
    /// Storage reads (key-value pairs per contract)
    #[serde(default)]
    pub storage: Vec<InitialStorageRead>,
    /// Nonce reads per contract
    #[serde(default)]
    pub nonces: Vec<InitialNonceRead>,
    /// Class hash lookups per contract
    #[serde(default)]
    pub class_hashes: Vec<InitialClassHashRead>,
    /// Class declaration checks
    #[serde(default)]
    pub declared_contracts: Vec<InitialDeclaredContract>,
}

/// Result of simulating a single transaction (MODIFIED in v0.10.1)
///
/// Now includes optional initial_reads when RETURN_INITIAL_READS flag is set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulateTransactionsResult {
    /// The fee estimation for this transaction
    pub fee_estimation: crate::v0_10_0::FeeEstimate,
    /// The execution trace of this transaction
    pub transaction_trace: TransactionTrace,
    /// Initial reads during execution (only present when RETURN_INITIAL_READS flag is set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_reads: Option<InitialReads>,
}

/// Result of simulating multiple transactions (NEW in v0.10.1)
///
/// Contains the results for each simulated transaction plus optional aggregate initial reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulateTransactionsResponse {
    /// Results for each simulated transaction
    pub simulated_transactions: Vec<SimulateTransactionsResult>,
    /// Aggregate initial reads for all transactions (only present when RETURN_INITIAL_READS flag is set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_reads: Option<InitialReads>,
}

/// A single pair of transaction hash and corresponding trace (MODIFIED in v0.10.1)
///
/// Now includes optional initial_reads when RETURN_INITIAL_READS flag is set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBlockTransactionsResult {
    /// The execution trace of the transaction
    pub trace_root: TransactionTrace,
    /// The hash of the transaction
    pub transaction_hash: Felt,
    /// Initial reads during execution (only present when RETURN_INITIAL_READS flag is set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_reads: Option<InitialReads>,
}

/// Response from traceBlockTransactions (NEW in v0.10.1)
///
/// Contains the traces for all transactions plus optional aggregate initial reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBlockTransactionsResponse {
    /// Traces for each transaction in the block
    pub traces: Vec<TraceBlockTransactionsResult>,
    /// Aggregate initial reads for all transactions (only present when RETURN_INITIAL_READS flag is set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_reads: Option<InitialReads>,
}
