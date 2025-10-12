use crate::custom_serde::NumAsHex;
pub use crate::v0_8_1::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockHeader, BlockNumber, BlockNumberHelper, BlockNumberParams, BroadcastedDeclareTxn, BroadcastedDeclareTxnV1,
    BroadcastedDeclareTxnV2, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, CallParams, ChainId, ChainIdParams, ClassAndTxnHash, ContractAbi, ContractAbiEntry,
    ContractAndTxnHash, ContractClass, ContractLeavesDataItem, ContractStorageDiffItem, ContractStorageKeysItem,
    ContractsProof, DaMode, DataAvailability, DeclareTxn, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2, DeclareTxnV3,
    DeployAccountTxn, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn, DeployedContractItem,
    DeprecatedCairoEntryPoint, DeprecatedContractClass, DeprecatedEntryPointsByType, EmittedEvent, EntryPointsByType,
    EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event, EventAbiEntry, EventAbiType, EventContent,
    EventsChunk, ExecutionResources, ExecutionStatus, FunctionAbiEntry, FunctionAbiType, FunctionCall,
    FunctionStateMutability, GetBlockTransactionCountParams, GetBlockWithReceiptsParams, GetBlockWithTxHashesParams,
    GetBlockWithTxsParams, GetClassAtParams, GetClassHashAtParams, GetClassParams, GetEventsParams, GetNonceParams,
    GetStateUpdateParams, GetStorageAtParams, GetStorageProofResult, GetTransactionByBlockIdAndIndexParams,
    GetTransactionByHashParams, GetTransactionReceiptParams, GetTransactionStatusParams, GlobalRoots, InvokeTxn,
    InvokeTxnV0, InvokeTxnV1, InvokeTxnV3, KeyValuePair, L1DaMode, L1HandlerTxn, MaybeDeprecatedContractClass,
    MerkleNode, MsgFromL1, MsgToL1, NewClasses, NodeHashToNodeMappingItem, NonceUpdate, ReplacedClass, ResourceBounds,
    ResourceBoundsMapping, ResourcePrice, SierraEntryPoint, Signature, SimulationFlagForEstimateFee, SpecVersionParams,
    StateDiff, StateUpdate, StorageKey, StructAbiEntry, StructAbiType, StructMember, SyncStatus, SyncingParams,
    SyncingStatus, Txn, TxnHash, TxnWithHash, TypedParameter,
};
use crate::v0_9_0::BlockId;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

/// fee payment info as it appears in receipts
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeePayment {
    /// amount paid
    pub amount: Felt,
    /// units in which the fee is given
    pub unit: PriceUnit,
}

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

/// receipt for l1 handler transaction
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

/// A tag specifying a dynamic reference to a block. Tag `l1_accepted` refers to the latest Starknet block which was included in a state update on L1 and finalized by the consensus on L1. Tag `latest` refers to the latest Starknet block finalized by the consensus on L2. Tag `pre_confirmed` refers to the block which is currently being built by the block proposer in height `latest` + 1.
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum BlockTag {
    #[serde(rename = "l1_accepted")]
    L1Accepted,
    #[serde(rename = "latest")]
    Latest,
    #[serde(rename = "pre_confirmed")]
    PreConfirmed,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EventFilterWithPageRequest {
    #[serde(default)]
    pub address: Option<Address>,
    #[serde(default)]
    pub from_block: Option<BlockId>,
    /// The values used to filter the events
    #[serde(default)]
    pub keys: Option<Vec<Vec<Felt>>>,
    #[serde(default)]
    pub to_block: Option<BlockId>,
    pub chunk_size: u64,
    /// The token returned from the previous query. If no token is provided the first page is returned.
    #[serde(default)]
    pub continuation_token: Option<String>,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePreConfirmedStateUpdate {
    Block(StateUpdate),
    PreConfirmed(PreConfirmedStateUpdate),
}
/// Pre-confirmed state update
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedStateUpdate {
    /// The previous global state root
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

/// The status of the block
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Copy, Debug)]
pub enum BlockStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TransactionAndReceipt {
    pub receipt: TxnReceipt,
    pub transaction: Txn,
}

/// The block object
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    pub status: BlockStatus,
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The block object
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithTxs {
    /// The transactions in this block
    pub transactions: Vec<TxnWithHash>,
    pub status: BlockStatus,
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The block object
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithTxHashes {
    /// The hashes of the transactions included in this block
    pub transactions: Vec<TxnHash>,
    pub status: BlockStatus,
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockHeader {
    /// specifies whether the data of this block is published via blob data or calldata
    pub l1_da_mode: L1DaMode,
    /// The price of l1 data gas in the block
    pub l1_data_gas_price: ResourcePrice,
    /// The price of l1 gas in the block
    pub l1_gas_price: ResourcePrice,
    /// The price of l2 gas in the block
    pub l2_gas_price: ResourcePrice,
    /// The block number of the block that the proposer is currently building. Note that this is a local view of the node, whose accuracy depends on its polling interval length.
    pub block_number: BlockNumber,
    /// The StarkNet identity of the sequencer submitting this block
    pub sequencer_address: Felt,
    /// Semver of the current Starknet protocol
    pub starknet_version: String,
    /// The time in which the block was created, encoded in Unix time
    pub timestamp: u64,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    #[serde(flatten)]
    pub pre_confirmed_block_header: PreConfirmedBlockHeader,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithTxs {
    /// The transactions in this block
    pub transactions: Vec<TxnWithHash>,
    #[serde(flatten)]
    pub pre_confirmed_block_header: PreConfirmedBlockHeader,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithTxHashes {
    /// The hashes of the transactions included in this block
    pub transactions: Vec<TxnHash>,
    #[serde(flatten)]
    pub pre_confirmed_block_header: PreConfirmedBlockHeader,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum StarknetGetBlockWithTxsAndReceiptsResult {
    Block(BlockWithReceipts),
    PreConfirmed(PreConfirmedBlockWithReceipts),
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithTxHashes {
    Block(BlockWithTxHashes),
    PreConfirmed(PreConfirmedBlockWithTxHashes),
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithTxs {
    Block(BlockWithTxs),
    PreConfirmed(PreConfirmedBlockWithTxs),
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

/// The finality status of the transaction, including the case the txn is still in the mempool or failed validation during the block construction phase
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum TxnStatus {
    #[serde(rename = "RECEIVED")]
    Received,
    #[serde(rename = "CANDIDATE")]
    Candidate,
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
}

/// The finality status of the transaction
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Copy, Debug)]
pub enum TxnFinalityStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    L2,
    #[serde(rename = "ACCEPTED_ON_L1")]
    L1,
}

impl From<BlockStatus> for TxnFinalityStatus {
    fn from(value: BlockStatus) -> Self {
        match value {
            BlockStatus::PreConfirmed => Self::PreConfirmed,
            BlockStatus::AcceptedOnL2 => Self::L2,
            BlockStatus::AcceptedOnL1 => Self::L1,
        }
    }
}

impl From<TxnFinalityStatus> for BlockStatus {
    fn from(value: TxnFinalityStatus) -> Self {
        match value {
            TxnFinalityStatus::PreConfirmed => Self::PreConfirmed,
            TxnFinalityStatus::L2 => Self::AcceptedOnL2,
            TxnFinalityStatus::L1 => Self::AcceptedOnL1,
        }
    }
}

/// The execution status of the transaction
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum TxnExecutionStatus {
    #[serde(rename = "REVERTED")]
    Reverted,
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TxnFinalityAndExecutionStatus {
    #[serde(default)]
    pub execution_status: Option<TxnExecutionStatus>,
    pub finality_status: TxnStatus,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum PriceUnitWei {
    #[serde(rename = "WEI")]
    #[default]
    Wei,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum PriceUnitFri {
    #[serde(rename = "FRI")]
    #[default]
    Fri,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum PriceUnit {
    Wei(PriceUnitWei),
    Fri(PriceUnitFri),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeeEstimateCommon {
    /// The Ethereum gas consumption of the transaction, charged for L1->L2 messages and, depending on the block's DA_MODE, state diffs
    #[serde(with = "NumAsHex")]
    pub l1_gas_consumed: u64,
    /// The gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    #[serde(with = "NumAsHex")]
    pub l1_gas_price: u128,
    /// The L2 gas consumption of the transaction
    #[serde(with = "NumAsHex")]
    pub l2_gas_consumed: u64,
    /// The L2 gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    #[serde(with = "NumAsHex")]
    pub l2_gas_price: u128,
    /// The Ethereum data gas consumption of the transaction
    #[serde(with = "NumAsHex")]
    pub l1_data_gas_consumed: u64,
    /// The data gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    #[serde(with = "NumAsHex")]
    pub l1_data_gas_price: u128,
    /// The estimated fee for the transaction (in wei or fri, depending on the tx version), equals to l1_gas_consumed*l1_gas_price + l1_data_gas_consumed*l1_data_gas_price + l2_gas_consumed*l2_gas_price
    #[serde(with = "NumAsHex")]
    pub overall_fee: u128,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeeEstimate {
    #[serde(flatten)]
    pub common: FeeEstimateCommon,
    /// Units in which the fee is given, can only be FRI
    pub unit: PriceUnitFri,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageFeeEstimate {
    #[serde(flatten)]
    pub common: FeeEstimateCommon,
    /// Units in which the fee is given, can only be WEI
    pub unit: PriceUnitWei,
}
