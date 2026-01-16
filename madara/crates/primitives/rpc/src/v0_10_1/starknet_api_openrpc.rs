// Re-export unchanged types from v0.10.0
pub use crate::v0_10_0::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockHeader, BlockNumber, BlockNumberHelper, BlockNumberParams, BlockStatus, BlockTag, BlockWithReceipts,
    BlockWithTxHashes, BlockWithTxs, BroadcastedDeclareTxn, BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2,
    BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn, CallParams, ChainId, ChainIdParams, ClassAndTxnHash,
    CommonReceiptProperties, ContractAbi, ContractAbiEntry, ContractAndTxnHash, ContractClass, ContractLeavesDataItem,
    ContractStorageDiffItem, ContractStorageKeysItem, ContractsProof, DaMode, DataAvailability, DeclareTxn,
    DeclareTxnReceipt, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2, DeclareTxnV3, DeployAccountTxn,
    DeployAccountTxnReceipt, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn, DeployTxnReceipt, DeployedContractItem,
    DeprecatedCairoEntryPoint, DeprecatedContractClass, DeprecatedEntryPointsByType, EntryPointsByType,
    EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event, EventAbiEntry, EventAbiType, EventContent,
    ExecutionResources, ExecutionStatus, FeeEstimate, FeeEstimateCommon, FeePayment, FunctionAbiEntry, FunctionAbiType,
    FunctionCall, FunctionStateMutability, GetBlockTransactionCountParams, GetBlockWithReceiptsParams,
    GetBlockWithTxHashesParams, GetBlockWithTxsParams, GetClassAtParams, GetClassHashAtParams, GetClassParams,
    GetNonceParams, GetStateUpdateParams, GetStorageAtParams, GetStorageProofResult,
    GetTransactionByBlockIdAndIndexParams, GetTransactionByHashParams, GetTransactionReceiptParams,
    GetTransactionStatusParams, GlobalRoots, InvokeTxn, InvokeTxnReceipt, InvokeTxnV0, InvokeTxnV1, KeyValuePair,
    L1DaMode, L1HandlerTxn, L1HandlerTxnReceipt, MaybeDeprecatedContractClass, MaybePreConfirmedBlockWithTxHashes,
    MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate, MerkleNode, MessageFeeEstimate, MigratedClassItem,
    MsgFromL1, MsgToL1, NewClasses, NodeHashToNodeMappingItem, NonceUpdate, PreConfirmedBlockHeader,
    PreConfirmedBlockWithReceipts, PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs, PreConfirmedStateUpdate,
    PriceUnitFri, PriceUnitWei, ReplacedClass, ResourceBounds, ResourceBoundsMapping, ResourcePrice, SierraEntryPoint,
    Signature, SimulationFlagForEstimateFee, SpecVersionParams, StarknetGetBlockWithTxsAndReceiptsResult, StateDiff,
    StateUpdate, StorageKey, StructAbiEntry, StructAbiType, StructMember, SyncStatus, SyncingParams, SyncingStatus,
    TransactionAndReceipt, Txn, TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnFinalityStatus, TxnHash,
    TxnReceipt, TxnReceiptWithBlockInfo, TxnStatus, TxnWithHash, TypedParameter,
};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;

/// Address filter supporting single address or array of addresses (NEW in v0.10.1)
///
/// This enum allows filtering events by either a single contract address
/// (for backward compatibility) or multiple addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AddressFilter {
    /// A single address filter (backward compatible)
    Single(Address),
    /// Multiple addresses filter (new in v0.10.1)
    Multiple(Vec<Address>),
}

impl AddressFilter {
    /// Convert the filter to a HashSet for efficient O(1) lookups during event filtering.
    /// Returns None if no addresses should be filtered (empty array matches all).
    pub fn to_set(&self) -> Option<HashSet<Felt>> {
        match self {
            AddressFilter::Single(addr) => Some(HashSet::from([*addr])),
            AddressFilter::Multiple(addrs) => {
                if addrs.is_empty() {
                    // Empty array means no filter (match all addresses)
                    None
                } else {
                    Some(addrs.iter().copied().collect())
                }
            }
        }
    }

    /// Check if this filter matches a given address.
    /// Empty array or None means match all.
    pub fn matches(&self, event_address: &Felt) -> bool {
        match self {
            AddressFilter::Single(addr) => addr == event_address,
            AddressFilter::Multiple(addrs) => addrs.is_empty() || addrs.contains(event_address),
        }
    }
}

/// Event filter with page request (MODIFIED in v0.10.1)
///
/// The `address` field now supports either a single address or an array of addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFilterWithPageRequest {
    /// The address filter - can be a single address or array of addresses (v0.10.1)
    #[serde(default)]
    pub address: Option<AddressFilter>,
    #[serde(default)]
    pub from_block: Option<crate::v0_10_0::BlockId>,
    /// The values used to filter the events
    #[serde(default)]
    pub keys: Option<Vec<Vec<Felt>>>,
    #[serde(default)]
    pub to_block: Option<crate::v0_10_0::BlockId>,
    pub chunk_size: u64,
    /// The token returned from the previous query. If no token is provided the first page is returned.
    #[serde(default)]
    pub continuation_token: Option<String>,
}

/// Re-export EmittedEvent from v0.10.0 (unchanged in v0.10.1)
pub use crate::v0_10_0::EmittedEvent;

/// Re-export EventsChunk from v0.10.0 (unchanged in v0.10.1)
pub use crate::v0_10_0::EventsChunk;

/// Get events params (MODIFIED in v0.10.1 to use new EventFilterWithPageRequest)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventsParams {
    pub filter: EventFilterWithPageRequest,
}

/// Proof fact element type (NEW in v0.10.1)
/// Represents a single proof fact associated with a transaction.
/// These are FELT values stored with the transaction.
pub type ProofFactElem = Felt;

/// Proof element type (NEW in v0.10.1)
/// Represents a proof element in broadcasted transactions.
/// According to spec: "type": "array", "items": { "type": "integer" }
/// These are integers passed when submitting transactions.
pub type ProofElem = u64;

/// INVOKE_TXN_V3 with optional proof_facts (NEW in v0.10.1)
///
/// For synced transactions that were submitted with proof facts to the gateway,
/// the proof_facts field will be populated. For transactions without proof facts
/// or old blocks, this field will be None.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxnV3 {
    /// The flattened base invoke v3 transaction fields
    #[serde(flatten)]
    pub inner: crate::v0_10_0::InvokeTxnV3,
    /// Optional proof facts for transactions submitted with proof (NEW in v0.10.1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_facts: Option<Vec<ProofFactElem>>,
}

/// Broadcasted INVOKE_TXN_V3 with optional proof (NEW in v0.10.1)
///
/// When submitting a transaction via addInvokeTransaction, users can optionally
/// include a proof that will be passed to the gateway.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastedInvokeTxnV3 {
    /// The flattened base broadcasted invoke v3 transaction fields
    #[serde(flatten)]
    pub inner: crate::v0_10_0::BroadcastedInvokeTxn,
    /// Optional proof to be passed to the gateway (NEW in v0.10.1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Vec<ProofElem>>,
}

/// Response flags for transaction-returning methods (NEW in v0.10.1)
///
/// Used with methods like getTransactionByHash, getBlockWithTxs to request
/// additional optional fields in the response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseFlag {
    /// Include proof_facts in INVOKE_TXN_V3 transactions
    #[serde(rename = "INCLUDE_PROOF_FACTS")]
    IncludeProofFacts,
}

/// Broadcasted invoke transaction (v0.10.1)
///
/// Re-export from v0.10.0 for backward compatibility, with the understanding that
/// BroadcastedInvokeTxnV3 now supports optional proof.
pub use crate::v0_10_0::BroadcastedInvokeTxn;

/// Broadcasted transaction enum (v0.10.1)
pub use crate::v0_10_0::BroadcastedTxn;

// ============================================================================
// v0.10.1 specific types for INCLUDE_PROOF_FACTS support
// ============================================================================
//
// These types mirror the v0.10.0 types but with InvokeTxnV3 containing proof_facts.
// Used when the INCLUDE_PROOF_FACTS response flag is set.
//
// For backward compatibility:
// - Old transactions stored without proof_facts will deserialize with proof_facts: None
// - When INCLUDE_PROOF_FACTS is NOT requested, use the re-exported v0.10.0 types
// - When INCLUDE_PROOF_FACTS IS requested, use these v0.10.1 specific types

/// Invoke transaction enum with proof_facts support (v0.10.1)
///
/// When INCLUDE_PROOF_FACTS is requested, V3 variant includes proof_facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum InvokeTxnWithProofFacts {
    /// Invoke transaction v0
    #[serde(rename = "0x0")]
    V0(crate::v0_10_0::InvokeTxnV0),
    /// Invoke transaction v1
    #[serde(rename = "0x1")]
    V1(crate::v0_10_0::InvokeTxnV1),
    /// Invoke transaction v3 with proof_facts
    #[serde(rename = "0x3")]
    V3(InvokeTxnV3),
}

/// Transaction enum with proof_facts support (v0.10.1)
///
/// When INCLUDE_PROOF_FACTS is requested, Invoke variant uses InvokeTxnWithProofFacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TxnWithProofFacts {
    /// Invoke transaction with proof_facts support
    #[serde(rename = "INVOKE")]
    Invoke(InvokeTxnWithProofFacts),
    /// L1 Handler transaction
    #[serde(rename = "L1_HANDLER")]
    L1Handler(crate::v0_10_0::L1HandlerTxn),
    /// Declare transaction
    #[serde(rename = "DECLARE")]
    Declare(crate::v0_10_0::DeclareTxn),
    /// Deploy transaction (legacy)
    #[serde(rename = "DEPLOY")]
    Deploy(crate::v0_10_0::DeployTxn),
    /// Deploy account transaction
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(crate::v0_10_0::DeployAccountTxn),
}

/// Transaction with hash and proof_facts support (v0.10.1)
///
/// Used when INCLUDE_PROOF_FACTS response flag is set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxnWithHashAndProofFacts {
    /// The transaction data with proof_facts support
    #[serde(flatten)]
    pub transaction: TxnWithProofFacts,
    /// The transaction hash
    pub transaction_hash: Felt,
}

// ============================================================================
// Block Types with proof_facts support (v0.10.1)
// ============================================================================
//
// These block types are returned when the INCLUDE_PROOF_FACTS response flag
// is set in methods like getBlockWithTxs, getTransactionByHash, etc.

/// Block with transactions including proof_facts (v0.10.1)
///
/// Used when INCLUDE_PROOF_FACTS response flag is set for getBlockWithTxs.
/// Contains transactions with optional proof_facts field for INVOKE_TXN_V3.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockWithTxsAndProofFacts {
    /// Block status
    pub status: BlockStatus,
    /// Block header
    #[serde(flatten)]
    pub block_header: crate::v0_10_0::BlockHeader,
    /// Transactions with proof_facts support
    pub transactions: Vec<TxnWithHashAndProofFacts>,
}

/// PreConfirmed block with transactions including proof_facts (v0.10.1)
///
/// Used for preconfirmed blocks when INCLUDE_PROOF_FACTS is set.
/// Unlike confirmed blocks, preconfirmed blocks don't have a status field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithTxsAndProofFacts {
    /// PreConfirmed block header (no block_hash or block_number)
    #[serde(flatten)]
    pub pre_confirmed_block_header: crate::v0_9_0::PreConfirmedBlockHeader,
    /// Transactions with proof_facts support
    pub transactions: Vec<TxnWithHashAndProofFacts>,
}

/// Result of getBlockWithTxs when INCLUDE_PROOF_FACTS is set (v0.10.1)
///
/// This enum distinguishes between confirmed and preconfirmed blocks,
/// both of which support proof_facts in their transactions.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithTxsAndProofFacts {
    /// A confirmed block with transactions and proof_facts
    Block(BlockWithTxsAndProofFacts),
    /// A preconfirmed block with transactions and proof_facts
    PreConfirmed(PreConfirmedBlockWithTxsAndProofFacts),
}
