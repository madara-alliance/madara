pub use crate::v0_9_0::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockNumber, BlockNumberHelper, BlockNumberParams, BlockStatus, BlockTag, BroadcastedDeclareTxn,
    BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn,
    BroadcastedInvokeTxn, BroadcastedTxn, CallParams, ChainId, ChainIdParams, ClassAndTxnHash, CommonReceiptProperties,
    ContractAbi, ContractAbiEntry, ContractAndTxnHash, ContractClass, ContractLeavesDataItem, ContractStorageDiffItem,
    ContractsProof, DaMode, DataAvailability, DeclareTxn, DeclareTxnReceipt, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2,
    DeclareTxnV3, DeployAccountTxn, DeployAccountTxnReceipt, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn,
    DeployTxnReceipt, DeployedContractItem, DeprecatedCairoEntryPoint, DeprecatedContractClass,
    DeprecatedEntryPointsByType, EntryPointsByType, EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event,
    EventAbiEntry, EventAbiType, EventContent, EventFilterWithPageRequest, ExecutionResources,
    ExecutionStatus, FeeEstimate, FeeEstimateCommon, FeePayment, FunctionAbiEntry, FunctionAbiType, FunctionCall,
    FunctionStateMutability, GetBlockTransactionCountParams, GetBlockWithReceiptsParams, GetBlockWithTxHashesParams,
    GetBlockWithTxsParams, GetClassAtParams, GetClassHashAtParams, GetClassParams, GetEventsParams, GetNonceParams,
    GetStateUpdateParams, GetStorageAtParams, GetStorageProofResult, GetTransactionByBlockIdAndIndexParams,
    GetTransactionByHashParams, GetTransactionReceiptParams, GetTransactionStatusParams, GlobalRoots, InvokeTxn,
    InvokeTxnReceipt, InvokeTxnV0, InvokeTxnV1, InvokeTxnV3, KeyValuePair, L1DaMode, L1HandlerTxn, L1HandlerTxnReceipt,
    MaybeDeprecatedContractClass, MerkleNode, MessageFeeEstimate, MsgFromL1, MsgToL1, NewClasses,
    NodeHashToNodeMappingItem, NonceUpdate, PreConfirmedBlockHeader, PreConfirmedBlockWithReceipts,
    PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs, PriceUnitFri, PriceUnitWei, ReplacedClass, ResourceBounds,
    ResourceBoundsMapping, ResourcePrice, SierraEntryPoint, Signature, SimulationFlagForEstimateFee, SpecVersionParams,
    StorageKey, StructAbiEntry, StructAbiType, StructMember, SyncStatus, SyncingParams, SyncingStatus,
    TransactionAndReceipt, Txn, TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnFinalityStatus, TxnHash,
    TxnReceipt, TxnReceiptWithBlockInfo, TxnStatus, TxnWithHash, TypedParameter,
};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

/// RPC 0.10.0 Changes:
/// 1. StateDiff: Added `migrated_compiled_classes` field
/// 2. PreConfirmedStateUpdate: Removed `old_root` field
/// 3. EmittedEvent: Added `transaction_index` and `event_index` fields
/// 4. ContractStorageKeysItem: Changed `storage_keys` type from `Vec<Felt>` to `Vec<StorageKey>`
///
/// The change in state applied in this block, given as a mapping of addresses to the new values and/or new contracts
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StateDiff {
    /// The declared class hash and compiled class hash
    pub declared_classes: Vec<NewClasses>,
    /// The deployed contracts
    pub deployed_contracts: Vec<DeployedContractItem>,
    /// The hash of the declared class
    pub deprecated_declared_classes: Vec<Felt>,
    /// The nonce updates
    pub nonces: Vec<NonceUpdate>,
    /// The replaced classes
    pub replaced_classes: Vec<ReplacedClass>,
    /// The storage diffs
    pub storage_diffs: Vec<ContractStorageDiffItem>,
    /// The migrated compiled classes (NEW in v0.10.0)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub migrated_compiled_classes: Vec<MigratedClassItem>,
}

/// A migrated class item representing a class that was migrated from Poseidon to BLAKE hash
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MigratedClassItem {
    /// The hash of the declared class
    pub class_hash: Felt,
    /// The new BLAKE hash (post-SNIP-34)
    pub compiled_class_hash: Felt,
}

/// State update for a confirmed block (v0.10.0: uses StateDiff with migrated_compiled_classes)
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StateUpdate {
    /// The hash of the block
    pub block_hash: BlockHash,
    /// The new global state root
    pub new_root: Felt,
    /// The previous global state root
    pub old_root: Felt,
    /// The state diff
    pub state_diff: StateDiff,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePreConfirmedStateUpdate {
    Block(StateUpdate),
    PreConfirmed(PreConfirmedStateUpdate),
}

/// Pre-confirmed state update (v0.10.0: removed `old_root` field)
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedStateUpdate {
    /// The state diff
    pub state_diff: StateDiff,
}

/// An event emitted as part of a transaction (v0.10.0: added transaction_index and event_index)
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EmittedEvent {
    /// The event information
    #[serde(flatten)]
    pub event: Event,
    /// The hash of the block in which the event was emitted
    #[serde(default)]
    pub block_hash: Option<BlockHash>,
    /// The number of the block in which the event was emitted
    #[serde(default)]
    pub block_number: Option<BlockNumber>,
    /// The transaction that emitted the event
    pub transaction_hash: TxnHash,
    /// The index of the transaction within the block
    pub transaction_index: u64,
    /// The index of the event within the transaction
    pub event_index: u64,
}

/// A chunk of events returned by getEvents (v0.10.0: uses EmittedEvent with event_index and transaction_index)
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EventsChunk {
    /// Use this token in a subsequent query to obtain the next page. Should not appear if there are no more pages.
    #[serde(default)]
    pub continuation_token: Option<String>,
    /// The events matching the filter
    pub events: Vec<EmittedEvent>,
}

/// Contract storage keys item (v0.10.0: changed storage_keys type from Vec<Felt> to Vec<StorageKey>)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractStorageKeysItem {
    /// The address of the contract
    pub contract_address: Felt,
    /// The storage keys (changed from Vec<Felt> to Vec<StorageKey> in v0.10.0)
    pub storage_keys: Vec<StorageKey>,
}

/// Block header (v0.10.0: added commitment and count fields)
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockHeader {
    /// The hash of this block
    pub block_hash: BlockHash,
    /// The block number (its height)
    pub block_number: BlockNumber,
    /// specifies whether the data of this block is published via blob data or calldata
    pub l1_da_mode: L1DaMode,
    /// The price of l1 data gas in the block
    pub l1_data_gas_price: ResourcePrice,
    /// The price of l1 gas in the block
    pub l1_gas_price: ResourcePrice,
    /// The price of l2 gas in the block
    pub l2_gas_price: ResourcePrice,
    /// The new global state root
    pub new_root: Felt,
    /// The hash of this block's parent
    pub parent_hash: BlockHash,
    /// The StarkNet identity of the sequencer submitting this block
    pub sequencer_address: Felt,
    /// Semver of the current Starknet protocol
    pub starknet_version: String,
    /// The time in which the block was created, encoded in Unix time
    pub timestamp: u64,
    /// A commitment to the events produced in this block (NEW in v0.10.0)
    pub event_commitment: Felt,
    /// A commitment to the transactions included in the block (NEW in v0.10.0)
    pub transaction_commitment: Felt,
    /// A commitment to the receipts produced in this block (NEW in v0.10.0)
    pub receipt_commitment: Felt,
    /// The state diff commitment hash in the block (NEW in v0.10.0)
    pub state_diff_commitment: Felt,
    /// The number of events in the block (NEW in v0.10.0)
    pub event_count: u64,
    /// The number of transactions in the block (NEW in v0.10.0)
    pub transaction_count: u64,
    /// The length of the state diff in the block (NEW in v0.10.0)
    pub state_diff_length: u64,
}

/// The block object with receipts
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    /// The status of the block
    pub status: BlockStatus,
    /// The block header
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The block object with full transactions
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithTxs {
    /// The transactions in this block
    pub transactions: Vec<TxnWithHash>,
    /// The status of the block
    pub status: BlockStatus,
    /// The block header
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The block object with transaction hashes
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithTxHashes {
    /// The hashes of the transactions included in this block
    pub transactions: Vec<TxnHash>,
    /// The status of the block
    pub status: BlockStatus,
    /// The block header
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// Result of getBlockWithReceipts
#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum StarknetGetBlockWithTxsAndReceiptsResult {
    Block(BlockWithReceipts),
    PreConfirmed(PreConfirmedBlockWithReceipts),
}

/// Result of getBlockWithTxHashes
#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithTxHashes {
    Block(BlockWithTxHashes),
    PreConfirmed(PreConfirmedBlockWithTxHashes),
}

/// Result of getBlockWithTxs
#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithTxs {
    Block(BlockWithTxs),
    PreConfirmed(PreConfirmedBlockWithTxs),
}
