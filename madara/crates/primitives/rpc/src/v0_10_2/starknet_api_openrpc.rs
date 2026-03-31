// Re-export unchanged types from v0.10.0
pub use crate::v0_10_0::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionResult, Address, BlockHash,
    BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper, BlockHeader, BlockNumber, BlockNumberHelper,
    BlockNumberParams, BlockStatus, BlockTag, BlockWithTxHashes, BlockWithTxs, BroadcastedDeclareTxn,
    BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn, CallParams,
    ChainId, ChainIdParams, ClassAndTxnHash, CommonReceiptProperties, ContractAbi, ContractAbiEntry,
    ContractAndTxnHash, ContractClass, ContractLeavesDataItem, ContractStorageDiffItem, ContractStorageKeysItem,
    ContractsProof, DaMode, DataAvailability, DeclareTxn, DeclareTxnReceipt, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2,
    DeclareTxnV3, DeployAccountTxn, DeployAccountTxnReceipt, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn,
    DeployTxnReceipt, DeployedContractItem, DeprecatedCairoEntryPoint, DeprecatedContractClass,
    DeprecatedEntryPointsByType, EntryPointsByType, EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event,
    EventAbiEntry, EventAbiType, EventContent, ExecutionResources, ExecutionStatus, FeeEstimate, FeeEstimateCommon,
    FeePayment, FunctionAbiEntry, FunctionAbiType, FunctionCall, FunctionStateMutability,
    GetBlockTransactionCountParams, GetBlockWithReceiptsParams, GetBlockWithTxHashesParams, GetBlockWithTxsParams,
    GetClassAtParams, GetClassHashAtParams, GetClassParams, GetNonceParams, GetStorageProofResult,
    GetTransactionByBlockIdAndIndexParams, GetTransactionByHashParams, GetTransactionReceiptParams,
    GetTransactionStatusParams, GlobalRoots, InvokeTxn, InvokeTxnReceipt, InvokeTxnV0, InvokeTxnV1, KeyValuePair,
    L1DaMode, L1HandlerTxn, L1HandlerTxnReceipt, MaybeDeprecatedContractClass, MaybePreConfirmedBlockWithTxHashes,
    MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate, MerkleNode, MessageFeeEstimate, MigratedClassItem,
    MsgFromL1, MsgToL1, NewClasses, NodeHashToNodeMappingItem, NonceUpdate, PreConfirmedBlockHeader,
    PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs, PreConfirmedStateUpdate, PriceUnitFri, PriceUnitWei,
    ReplacedClass, ResourceBounds, ResourceBoundsMapping, ResourcePrice, SierraEntryPoint, Signature,
    SimulationFlagForEstimateFee, SpecVersionParams, StateDiff, StateUpdate, StorageKey, StructAbiEntry, StructAbiType,
    StructMember, SyncStatus, SyncingParams, SyncingStatus, Txn, TxnExecutionStatus, TxnFinalityAndExecutionStatus,
    TxnFinalityStatus, TxnHash, TxnReceipt, TxnReceiptWithBlockInfo, TxnStatus, TxnWithHash, TypedParameter,
};
pub use crate::v0_9_0::{GetMessagesStatusParams, L1TxnHash, MessageStatus};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;

/// Address filter supporting single address or array of addresses (NEW in v0.10.2)
///
/// This enum allows filtering events by either a single contract address
/// (for backward compatibility) or multiple addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AddressFilter {
    /// A single address filter (backward compatible)
    Single(Address),
    /// Multiple addresses filter (new in v0.10.2)
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

/// Event filter with page request (MODIFIED in v0.10.2)
///
/// The `address` field now supports either a single address or an array of addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFilterWithPageRequest {
    /// The address filter - can be a single address or array of addresses (v0.10.2)
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

/// Re-export EmittedEvent from v0.10.0 (unchanged in v0.10.2)
pub use crate::v0_10_0::EmittedEvent;

/// Re-export EventsChunk from v0.10.0 (unchanged in v0.10.2)
pub use crate::v0_10_0::EventsChunk;

/// Get events params (MODIFIED in v0.10.2 to use new EventFilterWithPageRequest)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventsParams {
    pub filter: EventFilterWithPageRequest,
}

/// Proof fact element type (NEW in v0.10.2)
/// Represents a single proof fact associated with a transaction.
/// These are FELT values stored with the transaction.
pub type ProofFactElem = Felt;

/// Proof element type (NEW in v0.10.2)
/// Represents a proof element in broadcasted transactions.
/// According to spec: "type": "array", "items": { "type": "integer" }
/// These are integers passed when submitting transactions.
pub type ProofElem = u64;

/// INVOKE_TXN_V3 with optional proof_facts (NEW in v0.10.2)
///
/// For synced transactions that were submitted with proof facts to the gateway,
/// the proof_facts field will be populated. For transactions without proof facts
/// or old blocks, this field will be None.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxnV3 {
    /// The flattened base invoke v3 transaction fields
    #[serde(flatten)]
    pub inner: crate::v0_10_0::InvokeTxnV3,
    /// Optional proof facts for transactions submitted with proof (NEW in v0.10.2)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_facts: Option<Vec<ProofFactElem>>,
}

/// Broadcasted INVOKE_TXN_V3 with optional proof or proof_facts (NEW in v0.10.2)
///
/// When submitting a transaction via addInvokeTransaction, users can optionally
/// include either gateway proof inputs or already-expanded proof_facts. When both
/// are provided, proof_facts takes precedence.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BroadcastedInvokeTxnV3 {
    /// The flattened base broadcasted invoke v3 transaction fields
    #[serde(flatten)]
    pub inner: crate::v0_10_0::InvokeTxnV3,
    /// Optional proof to be passed to the gateway (NEW in v0.10.2)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Vec<ProofElem>>,
    /// Optional proof facts to use directly for replay/admin submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_facts: Option<Vec<ProofFactElem>>,
}

/// Response flags for transaction-returning methods (NEW in v0.10.2)
///
/// Used with methods like getTransactionByHash, getBlockWithTxs to request
/// additional optional fields in the response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseFlag {
    /// Include proof_facts in INVOKE_TXN_V3 transactions
    #[serde(rename = "INCLUDE_PROOF_FACTS")]
    IncludeProofFacts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageResponseFlag {
    #[serde(rename = "INCLUDE_LAST_UPDATE_BLOCK")]
    IncludeLastUpdateBlock,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageResult {
    pub value: Felt,
    pub last_update_block: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GetStorageAtResult {
    Value(Felt),
    Result(StorageResult),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStorageAtParams {
    pub contract_address: Address,
    pub key: StorageKey,
    pub block_id: crate::v0_10_0::BlockId,
    #[serde(default)]
    pub response_flags: Option<Vec<StorageResponseFlag>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStateUpdateParams {
    pub block_id: crate::v0_10_0::BlockId,
    #[serde(default)]
    pub contract_addresses: Option<Vec<Address>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedInvokeTxn {
    #[serde(rename = "0x0")]
    V0(crate::v0_10_0::InvokeTxnV0),
    #[serde(rename = "0x1")]
    V1(crate::v0_10_0::InvokeTxnV1),
    #[serde(rename = "0x3")]
    V3(BroadcastedInvokeTxnV3),

    #[serde(rename = "0x100000000000000000000000000000000")]
    QueryV0(crate::v0_10_0::InvokeTxnV0),
    #[serde(rename = "0x100000000000000000000000000000001")]
    QueryV1(crate::v0_10_0::InvokeTxnV1),
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(BroadcastedInvokeTxnV3),
}

impl BroadcastedInvokeTxn {
    pub fn is_query(&self) -> bool {
        matches!(self, Self::QueryV0(_) | Self::QueryV1(_) | Self::QueryV3(_))
    }

    pub fn version(&self) -> Felt {
        match self {
            Self::V0(_) | Self::QueryV0(_) => Felt::ZERO,
            Self::V1(_) | Self::QueryV1(_) => Felt::ONE,
            Self::V3(_) | Self::QueryV3(_) => Felt::THREE,
        }
    }
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum BroadcastedTxn {
    #[serde(rename = "INVOKE")]
    Invoke(BroadcastedInvokeTxn),
    #[serde(rename = "DECLARE")]
    Declare(BroadcastedDeclareTxn),
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(BroadcastedDeployAccountTxn),
}

impl BroadcastedTxn {
    pub fn is_query(&self) -> bool {
        match self {
            Self::Invoke(txn) => txn.is_query(),
            Self::Declare(txn) => txn.is_query(),
            Self::DeployAccount(txn) => txn.is_query(),
        }
    }

    pub fn version(&self) -> Felt {
        match self {
            Self::Invoke(txn) => txn.version(),
            Self::Declare(txn) => txn.version(),
            Self::DeployAccount(txn) => txn.version(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AddInvokeTransactionParams {
    pub invoke_transaction: BroadcastedInvokeTxn,
}

impl From<crate::v0_9_0::BroadcastedInvokeTxn> for BroadcastedInvokeTxn {
    fn from(value: crate::v0_9_0::BroadcastedInvokeTxn) -> Self {
        match value {
            crate::v0_9_0::BroadcastedInvokeTxn::V0(tx) => Self::V0(tx),
            crate::v0_9_0::BroadcastedInvokeTxn::V1(tx) => Self::V1(tx),
            crate::v0_9_0::BroadcastedInvokeTxn::V3(tx) => {
                Self::V3(BroadcastedInvokeTxnV3 { inner: tx, proof: None, proof_facts: None })
            }
            crate::v0_9_0::BroadcastedInvokeTxn::QueryV0(tx) => Self::QueryV0(tx),
            crate::v0_9_0::BroadcastedInvokeTxn::QueryV1(tx) => Self::QueryV1(tx),
            crate::v0_9_0::BroadcastedInvokeTxn::QueryV3(tx) => {
                Self::QueryV3(BroadcastedInvokeTxnV3 { inner: tx, proof: None, proof_facts: None })
            }
        }
    }
}

impl From<crate::v0_7_1::BroadcastedInvokeTxn> for BroadcastedInvokeTxn {
    fn from(value: crate::v0_7_1::BroadcastedInvokeTxn) -> Self {
        crate::v0_8_1::BroadcastedInvokeTxn::from(value).into()
    }
}

impl From<BroadcastedInvokeTxnV3> for crate::v0_8_1::InvokeTxnV3 {
    fn from(value: BroadcastedInvokeTxnV3) -> Self {
        value.inner
    }
}

impl From<BroadcastedInvokeTxn> for crate::v0_8_1::BroadcastedInvokeTxn {
    fn from(value: BroadcastedInvokeTxn) -> Self {
        match value {
            BroadcastedInvokeTxn::V0(tx) => Self::V0(tx),
            BroadcastedInvokeTxn::V1(tx) => Self::V1(tx),
            BroadcastedInvokeTxn::V3(tx) => Self::V3(tx.into()),
            BroadcastedInvokeTxn::QueryV0(tx) => Self::QueryV0(tx),
            BroadcastedInvokeTxn::QueryV1(tx) => Self::QueryV1(tx),
            BroadcastedInvokeTxn::QueryV3(tx) => Self::QueryV3(tx.into()),
        }
    }
}

impl From<BroadcastedTxn> for crate::v0_8_1::BroadcastedTxn {
    fn from(value: BroadcastedTxn) -> Self {
        match value {
            BroadcastedTxn::Invoke(tx) => Self::Invoke(tx.into()),
            BroadcastedTxn::Declare(tx) => Self::Declare(tx),
            BroadcastedTxn::DeployAccount(tx) => Self::DeployAccount(tx),
        }
    }
}

// ============================================================================
// v0.10.2 specific types for INCLUDE_PROOF_FACTS support
// ============================================================================
//
// These types mirror the v0.10.0 types but with InvokeTxnV3 containing proof_facts.
// Used when the INCLUDE_PROOF_FACTS response flag is set.
//
// For backward compatibility:
// - Old transactions stored without proof_facts will deserialize with proof_facts: None
// - When INCLUDE_PROOF_FACTS is NOT requested, use the re-exported v0.10.0 types
// - When INCLUDE_PROOF_FACTS IS requested, use these v0.10.2 specific types

/// Invoke transaction enum with proof_facts support (v0.10.2)
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

/// Transaction enum with proof_facts support (v0.10.2)
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

/// Transaction with hash and proof_facts support (v0.10.2)
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

/// Transaction and receipt pair with proof_facts-aware transaction (v0.10.2)
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionAndReceipt {
    pub receipt: TxnReceipt,
    pub transaction: TxnWithProofFacts,
}

// ============================================================================
// Block Types with proof_facts support (v0.10.2)
// ============================================================================
//
// These block types are returned when the INCLUDE_PROOF_FACTS response flag
// is set in methods like getBlockWithTxs, getTransactionByHash, etc.

/// Block with transactions including proof_facts (v0.10.2)
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

/// PreConfirmed block with transactions including proof_facts (v0.10.2)
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

/// Result of getBlockWithTxs when INCLUDE_PROOF_FACTS is set (v0.10.2)
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

/// The block object with receipts (v0.10.2)
///
/// Uses proof_facts-aware transactions when INCLUDE_PROOF_FACTS is requested.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    /// The status of the block
    pub status: BlockStatus,
    /// The block header
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The dynamic block being constructed by the sequencer (v0.10.2)
///
/// Uses proof_facts-aware transactions when INCLUDE_PROOF_FACTS is requested.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    #[serde(flatten)]
    pub pre_confirmed_block_header: PreConfirmedBlockHeader,
}

/// Result of getBlockWithReceipts (v0.10.2)
#[allow(clippy::large_enum_variant)]
#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum StarknetGetBlockWithTxsAndReceiptsResult {
    Block(BlockWithReceipts),
    PreConfirmed(PreConfirmedBlockWithReceipts),
}
