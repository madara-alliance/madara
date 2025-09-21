use crate::custom_serde::NumAsHex;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub use crate::v0_7_1::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockId, BlockNumber, BlockNumberHelper, BlockNumberParams, BlockStatus, BlockTag, BroadcastedDeclareTxnV1,
    BroadcastedDeclareTxnV2, CallParams, Calldata, ChainId, ChainIdParams, ClassAndTxnHash, ContractAbi,
    ContractAbiEntry, ContractAndTxnHash, ContractClass, ContractStorageDiffItem, DaMode, DataAvailability,
    DeclareTxnV0, DeclareTxnV1, DeclareTxnV2, DeployAccountTxnV1, DeployTxn, DeployedContractItem,
    DeprecatedCairoEntryPoint, DeprecatedContractClass, DeprecatedEntryPointsByType, EmittedEvent, EntryPointsByType,
    EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event, EventAbiEntry, EventAbiType, EventContent,
    EventFilterWithPageRequest, EventsChunk, ExecutionStatus, FeePayment, FunctionAbiEntry, FunctionAbiType,
    FunctionCall, FunctionStateMutability, GetBlockTransactionCountParams, GetBlockWithReceiptsParams,
    GetBlockWithTxHashesParams, GetBlockWithTxsParams, GetClassAtParams, GetClassHashAtParams, GetClassParams,
    GetEventsParams, GetNonceParams, GetStateUpdateParams, GetStorageAtParams, GetTransactionByBlockIdAndIndexParams,
    GetTransactionByHashParams, GetTransactionReceiptParams, GetTransactionStatusParams, InvokeTxnV0, InvokeTxnV1,
    KeyValuePair, L1DaMode, L1HandlerTxn, MaybeDeprecatedContractClass, MaybePendingStateUpdate, MsgFromL1, MsgToL1,
    NewClasses, NonceUpdate, PendingStateUpdate, PriceUnit, ReplacedClass, ResourceBounds, ResourcePrice,
    SierraEntryPoint, Signature, SimulationFlagForEstimateFee, SpecVersionParams, StateDiff, StateUpdate, StorageKey,
    StructAbiEntry, StructAbiType, StructMember, SyncStatus, SyncingParams, SyncingStatus, TxnExecutionStatus,
    TxnFinalityAndExecutionStatus, TxnFinalityStatus, TxnHash, TxnStatus, TypedParameter,
};

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

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedInvokeTxn {
    #[serde(rename = "0x0")]
    V0(InvokeTxnV0),
    #[serde(rename = "0x1")]
    V1(InvokeTxnV1),
    #[serde(rename = "0x3")]
    V3(InvokeTxnV3),

    /// Query-only broadcasted invoke transaction.
    #[serde(rename = "0x100000000000000000000000000000000")]
    QueryV0(InvokeTxnV0),
    /// Query-only broadcasted invoke transaction.
    #[serde(rename = "0x100000000000000000000000000000001")]
    QueryV1(InvokeTxnV1),
    /// Query-only broadcasted invoke transaction.
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(InvokeTxnV3),
}

impl BroadcastedInvokeTxn {
    pub fn is_query(&self) -> bool {
        match self {
            Self::QueryV0(_) | Self::QueryV1(_) | Self::QueryV3(_) => true,
            Self::V0(_) | Self::V1(_) | Self::V3(_) => false,
        }
    }
    pub fn version(&self) -> Felt {
        match self {
            Self::V0(_) | Self::QueryV0(_) => Felt::ZERO,
            Self::V1(_) | Self::QueryV1(_) => Felt::ONE,
            Self::V3(_) | Self::QueryV3(_) => Felt::THREE,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedDeclareTxn {
    #[serde(rename = "0x1")]
    V1(BroadcastedDeclareTxnV1),
    #[serde(rename = "0x2")]
    V2(BroadcastedDeclareTxnV2),
    #[serde(rename = "0x3")]
    V3(BroadcastedDeclareTxnV3),

    /// Query-only broadcasted declare transaction.
    #[serde(rename = "0x100000000000000000000000000000001")]
    QueryV1(BroadcastedDeclareTxnV1),
    /// Query-only broadcasted declare transaction.
    #[serde(rename = "0x100000000000000000000000000000002")]
    QueryV2(BroadcastedDeclareTxnV2),
    /// Query-only broadcasted declare transaction.
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(BroadcastedDeclareTxnV3),
}

impl BroadcastedDeclareTxn {
    pub fn is_query(&self) -> bool {
        match self {
            Self::QueryV1(_) | Self::QueryV2(_) | Self::QueryV3(_) => true,
            Self::V1(_) | Self::V2(_) | Self::V3(_) => false,
        }
    }
    pub fn version(&self) -> Felt {
        match self {
            Self::V1(_) | Self::QueryV1(_) => Felt::ONE,
            Self::V2(_) | Self::QueryV2(_) => Felt::TWO,
            Self::V3(_) | Self::QueryV3(_) => Felt::THREE,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedDeployAccountTxn {
    #[serde(rename = "0x1")]
    V1(DeployAccountTxnV1),
    #[serde(rename = "0x3")]
    V3(DeployAccountTxnV3),

    /// Query-only broadcasted deploy account transaction.
    #[serde(rename = "0x100000000000000000000000000000001")]
    QueryV1(DeployAccountTxnV1),
    /// Query-only broadcasted deploy account transaction.
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(DeployAccountTxnV3),
}

impl BroadcastedDeployAccountTxn {
    pub fn is_query(&self) -> bool {
        match self {
            Self::QueryV1(_) | Self::QueryV3(_) => true,
            Self::V1(_) | Self::V3(_) => false,
        }
    }
    pub fn version(&self) -> Felt {
        match self {
            Self::V1(_) | Self::QueryV1(_) => Felt::ONE,
            Self::V3(_) | Self::QueryV3(_) => Felt::THREE,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BroadcastedDeclareTxnV3 {
    /// data needed to deploy the account contract from which this tx will be initiated
    pub account_deployment_data: Vec<Felt>,
    /// The hash of the Cairo assembly resulting from the Sierra compilation
    pub compiled_class_hash: Felt,
    /// The class to be declared
    pub contract_class: ContractClass,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DaMode,
    pub nonce: Felt,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode)
    pub nonce_data_availability_mode: DaMode,
    /// data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
    /// the tip for the transaction
    #[serde(with = "NumAsHex")]
    pub tip: u64,
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
    /// If this field is missing, it means the receipt belongs to the pending block
    #[serde(default)]
    pub block_hash: Option<BlockHash>,
    /// If this field is missing, it means the receipt belongs to the pending block
    #[serde(default)]
    pub block_number: Option<BlockNumber>,
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
pub struct TxnWithHash {
    #[serde(flatten)]
    pub transaction: Txn,
    pub transaction_hash: TxnHash,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockHeader {
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
pub struct PendingBlockHeader {
    /// specifies whether the data of this block is published via blob data or calldata
    pub l1_da_mode: L1DaMode,
    /// The price of l1 data gas in the block
    pub l1_data_gas_price: ResourcePrice,
    /// The price of l1 gas in the block
    pub l1_gas_price: ResourcePrice,
    /// The price of l2 gas in the block
    pub l2_gas_price: ResourcePrice,
    /// The hash of this block's parent
    pub parent_hash: BlockHash,
    /// The StarkNet identity of the sequencer submitting this block
    pub sequencer_address: Felt,
    /// Semver of the current Starknet protocol
    pub starknet_version: String,
    /// The time in which the block was created, encoded in Unix time
    pub timestamp: u64,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingBlockWithReceipts {
    /// The transactions in this block
    pub transactions: Vec<TransactionAndReceipt>,
    #[serde(flatten)]
    pub pending_block_header: PendingBlockHeader,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingBlockWithTxs {
    /// The transactions in this block
    pub transactions: Vec<TxnWithHash>,
    #[serde(flatten)]
    pub pending_block_header: PendingBlockHeader,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingBlockWithTxHashes {
    /// The hashes of the transactions included in this block
    pub transactions: Vec<TxnHash>,
    #[serde(flatten)]
    pub pending_block_header: PendingBlockHeader,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum StarknetGetBlockWithTxsAndReceiptsResult {
    Block(BlockWithReceipts),
    Pending(PendingBlockWithReceipts),
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePendingBlockWithTxHashes {
    Block(BlockWithTxHashes),
    Pending(PendingBlockWithTxHashes),
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybePendingBlockWithTxs {
    Block(BlockWithTxs),
    Pending(PendingBlockWithTxs),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeeEstimate {
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

/// the resources consumed by the transaction, includes both computation and data
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResources {
    pub l1_gas: u128,
    pub l2_gas: u128,
    pub l1_data_gas: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractStorageKeysItem {
    pub contract_address: Felt,
    pub storage_keys: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MerkleNode {
    Binary { left: Felt, right: Felt },
    Edge { child: Felt, path: Felt, length: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHashToNodeMappingItem {
    pub node_hash: Felt,
    pub node: MerkleNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractLeavesDataItem {
    pub nonce: Felt,
    pub class_hash: Felt,
    pub storage_root: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractsProof {
    pub nodes: Vec<NodeHashToNodeMappingItem>,
    pub contract_leaves_data: Vec<ContractLeavesDataItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalRoots {
    pub contracts_tree_root: Felt,
    pub classes_tree_root: Felt,
    pub block_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStorageProofResult {
    pub classes_proof: Vec<NodeHashToNodeMappingItem>,
    pub contracts_proof: ContractsProof,
    pub contracts_storage_proofs: Vec<Vec<NodeHashToNodeMappingItem>>,
    pub global_roots: GlobalRoots,
}

/// The transaction schema, as it appears inside a block
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Txn {
    #[serde(rename = "INVOKE")]
    Invoke(InvokeTxn),
    #[serde(rename = "L1_HANDLER")]
    L1Handler(L1HandlerTxn),
    #[serde(rename = "DECLARE")]
    Declare(DeclareTxn),
    #[serde(rename = "DEPLOY")]
    Deploy(DeployTxn),
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(DeployAccountTxn),
}

/// Initiate a transaction from an account
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "version")]
pub enum InvokeTxn {
    #[serde(rename = "0x0")]
    V0(InvokeTxnV0),
    #[serde(rename = "0x1")]
    V1(InvokeTxnV1),
    #[serde(rename = "0x3")]
    V3(InvokeTxnV3),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct InvokeTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct InvokeTxnV3 {
    /// data needed to deploy the account contract from which this tx will be initiated
    pub account_deployment_data: Vec<Felt>,
    /// The data expected by the account's `execute` function (in most usecases, this includes the called contract address and a function selector)
    pub calldata: Calldata,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DaMode,
    pub nonce: Felt,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode)
    pub nonce_data_availability_mode: DaMode,
    /// data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    pub sender_address: Address,
    pub signature: Signature,
    /// the tip for the transaction
    #[serde(with = "NumAsHex")]
    pub tip: u64,
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
pub struct DeployTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
    /// The address of the deployed contract
    pub contract_address: Felt,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "version")]
pub enum DeclareTxn {
    #[serde(rename = "0x0")]
    V0(DeclareTxnV0),
    #[serde(rename = "0x1")]
    V1(DeclareTxnV1),
    #[serde(rename = "0x2")]
    V2(DeclareTxnV2),
    #[serde(rename = "0x3")]
    V3(DeclareTxnV3),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeclareTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeclareTxnV3 {
    /// data needed to deploy the account contract from which this tx will be initiated
    pub account_deployment_data: Vec<Felt>,
    /// The hash of the declared class
    pub class_hash: Felt,
    /// The hash of the Cairo assembly resulting from the Sierra compilation
    pub compiled_class_hash: Felt,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DaMode,
    pub nonce: Felt,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode)
    pub nonce_data_availability_mode: DaMode,
    /// data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
    /// the tip for the transaction
    #[serde(with = "NumAsHex")]
    pub tip: u64,
}
/// deploys a new account contract
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "version")]
pub enum DeployAccountTxn {
    #[serde(rename = "0x1")]
    V1(DeployAccountTxnV1),
    #[serde(rename = "0x3")]
    V3(DeployAccountTxnV3),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployAccountTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
    /// The address of the deployed contract
    pub contract_address: Felt,
}

/// Deploys an account contract, charges fee from the pre-funded account addresses
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployAccountTxnV3 {
    /// The hash of the deployed contract's class
    pub class_hash: Felt,
    /// The parameters passed to the constructor
    pub constructor_calldata: Vec<Felt>,
    /// The salt for the address of the deployed contract
    pub contract_address_salt: Felt,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DaMode,
    pub nonce: Felt,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode)
    pub nonce_data_availability_mode: DaMode,
    /// data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    pub signature: Signature,
    /// the tip for the transaction
    #[serde(with = "NumAsHex")]
    pub tip: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ResourceBoundsMapping {
    /// The max amount and max price per unit of L1 gas used in this tx
    pub l1_gas: ResourceBounds,
    /// The max amount and max price per unit of L2 gas used in this tx
    pub l2_gas: ResourceBounds,
    /// The max amount and max price per unit of L1 data gas used in this tx
    pub l1_data_gas: ResourceBounds,
}
