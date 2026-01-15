// Re-export unchanged types from v0.10.0
pub use crate::v0_10_0::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockHeader, BlockNumber, BlockNumberHelper, BlockNumberParams, BlockStatus, BlockTag, BlockWithReceipts,
    BlockWithTxHashes, BlockWithTxs, BroadcastedDeclareTxn, BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2,
    BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn, CallParams, ChainId, ChainIdParams, ClassAndTxnHash,
    CommonReceiptProperties, ContractAbi, ContractAbiEntry, ContractAndTxnHash, ContractClass,
    ContractLeavesDataItem, ContractStorageDiffItem, ContractStorageKeysItem, ContractsProof, DaMode,
    DataAvailability, DeclareTxn, DeclareTxnReceipt, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2, DeclareTxnV3,
    DeployAccountTxn, DeployAccountTxnReceipt, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn, DeployTxnReceipt,
    DeployedContractItem, DeprecatedCairoEntryPoint, DeprecatedContractClass, DeprecatedEntryPointsByType,
    EntryPointsByType, EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event, EventAbiEntry, EventAbiType,
    EventContent, ExecutionResources, ExecutionStatus, FeeEstimate, FeeEstimateCommon, FeePayment, FunctionAbiEntry,
    FunctionAbiType, FunctionCall, FunctionStateMutability, GetBlockTransactionCountParams,
    GetBlockWithReceiptsParams, GetBlockWithTxHashesParams, GetBlockWithTxsParams, GetClassAtParams,
    GetClassHashAtParams, GetClassParams, GetNonceParams, GetStateUpdateParams, GetStorageAtParams,
    GetStorageProofResult, GetTransactionByBlockIdAndIndexParams, GetTransactionByHashParams,
    GetTransactionReceiptParams, GetTransactionStatusParams, GlobalRoots, InvokeTxn, InvokeTxnReceipt, InvokeTxnV0,
    InvokeTxnV1, KeyValuePair, L1DaMode, L1HandlerTxn, L1HandlerTxnReceipt, MaybeDeprecatedContractClass,
    MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate, MerkleNode,
    MessageFeeEstimate, MigratedClassItem, MsgFromL1, MsgToL1, NewClasses, NodeHashToNodeMappingItem, NonceUpdate,
    PreConfirmedBlockHeader, PreConfirmedBlockWithReceipts, PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs,
    PreConfirmedStateUpdate, PriceUnitFri, PriceUnitWei, ReplacedClass, ResourceBounds, ResourceBoundsMapping,
    ResourcePrice, SierraEntryPoint, Signature, SimulationFlagForEstimateFee, SpecVersionParams, StateDiff,
    StateUpdate, StarknetGetBlockWithTxsAndReceiptsResult, StorageKey, StructAbiEntry, StructAbiType, StructMember,
    SyncStatus, SyncingParams, SyncingStatus, TransactionAndReceipt, Txn, TxnExecutionStatus,
    TxnFinalityAndExecutionStatus, TxnFinalityStatus, TxnHash, TxnReceipt, TxnReceiptWithBlockInfo, TxnStatus,
    TxnWithHash, TypedParameter,
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
/// Represents a single proof fact associated with a transaction
pub type ProofFactElem = Felt;

/// Proof element type (NEW in v0.10.1)
/// Represents a proof element in broadcasted transactions
pub type ProofElem = Felt;

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
