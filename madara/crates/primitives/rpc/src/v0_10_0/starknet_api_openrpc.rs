pub use crate::v0_9_0::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockNumber, BlockNumberHelper, BlockNumberParams, BlockStatus, BlockTag, BroadcastedDeclareTxn,
    BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn,
    BroadcastedInvokeTxn, BroadcastedTxn, CallParams, ChainId, ChainIdParams, ClassAndTxnHash, ContractAbi,
    ContractAbiEntry, ContractAndTxnHash, ContractClass, ContractLeavesDataItem, ContractStorageDiffItem,
    ContractsProof, DaMode, DataAvailability, DeclareTxn, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2, DeclareTxnV3,
    DeployAccountTxn, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn, DeployedContractItem,
    DeprecatedCairoEntryPoint, DeprecatedContractClass, DeprecatedEntryPointsByType, EntryPointsByType,
    EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event, EventAbiEntry, EventAbiType, EventContent,
    EventFilterWithPageRequest, ExecutionStatus, FeeEstimate, FeeEstimateCommon, FeePayment, FunctionAbiEntry,
    FunctionAbiType, FunctionCall, FunctionStateMutability, GetBlockTransactionCountParams, GetBlockWithReceiptsParams,
    GetBlockWithTxHashesParams, GetBlockWithTxsParams, GetClassAtParams, GetClassHashAtParams, GetClassParams,
    GetEventsParams, GetNonceParams, GetStateUpdateParams, GetStorageAtParams, GetStorageProofResult,
    GetTransactionByBlockIdAndIndexParams, GetTransactionByHashParams, GetTransactionReceiptParams,
    GetTransactionStatusParams, GlobalRoots, InvokeTxn, InvokeTxnV0, InvokeTxnV1, InvokeTxnV3, KeyValuePair, L1DaMode,
    L1HandlerTxn, MaybeDeprecatedContractClass, MerkleNode, MessageFeeEstimate, MsgFromL1, MsgToL1, NewClasses,
    NodeHashToNodeMappingItem, NonceUpdate, PreConfirmedBlockHeader, PreConfirmedBlockWithTxHashes,
    PreConfirmedBlockWithTxs, PriceUnit, PriceUnitFri, PriceUnitWei, ReplacedClass, ResourceBounds,
    ResourceBoundsMapping, ResourcePrice, SierraEntryPoint, Signature, SimulationFlagForEstimateFee, SpecVersionParams,
    StorageKey, StructAbiEntry, StructAbiType, StructMember, SyncStatus, SyncingParams, SyncingStatus, Txn,
    TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnFinalityStatus, TxnHash, TxnStatus, TxnWithHash,
    TypedParameter,
};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

/// RPC 0.10.0 Changes:
/// 1. StateDiff: Added `migrated_compiled_classes` field
/// 2. PreConfirmedStateUpdate: Removed `old_root` field
/// 3. EmittedEvent: Added `transaction_index` and `event_index` fields
/// 4. ContractStorageKeysItem: Changed `storage_keys` type from `Vec<Felt>` to `Vec<StorageKey>`
/// 5. Receipt `execution_resources` values are plain integers
///
/// The resources consumed by the transaction.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResources {
    pub l1_gas: u64,
    pub l1_data_gas: u64,
    pub l2_gas: u64,
}

impl From<crate::v0_8_1::ExecutionResources> for ExecutionResources {
    fn from(resources: crate::v0_8_1::ExecutionResources) -> Self {
        Self {
            l1_gas: resources.l1_gas.try_into().unwrap_or(u64::MAX),
            l1_data_gas: resources.l1_data_gas.try_into().unwrap_or(u64::MAX),
            l2_gas: resources.l2_gas.try_into().unwrap_or(u64::MAX),
        }
    }
}

/// Shared receipt fields.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct CommonReceiptProperties {
    /// The fee that was charged by the sequencer
    pub actual_fee: FeePayment,
    /// The events emitted as part of this transaction
    pub events: Vec<Event>,
    /// The resources consumed by the transaction
    pub execution_resources: ExecutionResources,
    /// finality status of the tx
    pub finality_status: TxnFinalityStatus,
    pub messages_sent: Vec<MsgToL1>,
    /// The hash identifying the transaction
    pub transaction_hash: TxnHash,
    #[serde(flatten)]
    pub execution_status: ExecutionStatus,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum TxnReceipt {
    #[serde(rename = "INVOKE")]
    Invoke(InvokeTxnReceipt),
    #[serde(rename = "L1_HANDLER")]
    L1Handler(L1HandlerTxnReceipt),
    #[serde(rename = "DECLARE")]
    Declare(DeclareTxnReceipt),
    #[serde(rename = "DEPLOY")]
    Deploy(DeployTxnReceipt),
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(DeployAccountTxnReceipt),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct InvokeTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
}

/// Receipt for l1 handler transaction.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct L1HandlerTxnReceipt {
    /// The message hash as it appears on the L1 core contract
    pub message_hash: String,
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeclareTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployAccountTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
    /// The address of the deployed contract
    pub contract_address: Felt,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
    /// The address of the deployed contract
    pub contract_address: Felt,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TransactionAndReceipt {
    pub receipt: TxnReceipt,
    pub transaction: Txn,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TxnReceiptWithBlockInfo {
    #[serde(flatten)]
    pub transaction_receipt: TxnReceipt,
    /// If this field is missing, it means the receipt belongs to the pre-confirmed block
    #[serde(default)]
    pub block_hash: Option<BlockHash>,
    pub block_number: BlockNumber,
}
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
    #[serde(default)]
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

/// The dynamic block being constructed by the sequencer.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    #[serde(flatten)]
    pub pre_confirmed_block_header: PreConfirmedBlockHeader,
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
