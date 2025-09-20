use super::*;
use crate::custom_serde::NumAsHex;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub type Address = Felt;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TransactionAndReceipt {
    pub receipt: TxnReceipt,
    pub transaction: Txn,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TxnWithHash {
    #[serde(flatten)]
    pub transaction: Txn,
    pub transaction_hash: TxnHash,
}

pub type BlockHash = Felt;

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

/// The block's number (its height)
pub type BlockNumber = u64;

/// The status of the block
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum BlockStatus {
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "REJECTED")]
    Rejected,
}

/// A tag specifying a dynamic reference to a block
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum BlockTag {
    #[serde(rename = "latest")]
    Latest,
    #[serde(rename = "pending")]
    Pending,
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
pub struct BroadcastedDeclareTxnV1 {
    /// The class to be declared
    pub contract_class: DeprecatedContractClass,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub nonce: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BroadcastedDeclareTxnV2 {
    /// The hash of the Cairo assembly resulting from the Sierra compilation
    pub compiled_class_hash: Felt,
    /// The class to be declared
    pub contract_class: ContractClass,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub nonce: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
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
            BroadcastedTxn::Invoke(txn) => txn.is_query(),
            BroadcastedTxn::Declare(txn) => txn.is_query(),
            BroadcastedTxn::DeployAccount(txn) => txn.is_query(),
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
            BroadcastedInvokeTxn::QueryV0(_) | BroadcastedInvokeTxn::QueryV1(_) | BroadcastedInvokeTxn::QueryV3(_) => {
                true
            }
            BroadcastedInvokeTxn::V0(_) | BroadcastedInvokeTxn::V1(_) | BroadcastedInvokeTxn::V3(_) => false,
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
            BroadcastedDeclareTxn::QueryV1(_)
            | BroadcastedDeclareTxn::QueryV2(_)
            | BroadcastedDeclareTxn::QueryV3(_) => true,
            BroadcastedDeclareTxn::V1(_) | BroadcastedDeclareTxn::V2(_) | BroadcastedDeclareTxn::V3(_) => false,
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
            BroadcastedDeployAccountTxn::QueryV1(_) | BroadcastedDeployAccountTxn::QueryV3(_) => true,
            BroadcastedDeployAccountTxn::V1(_) | BroadcastedDeployAccountTxn::V3(_) => false,
        }
    }
}

/// StarkNet chain id, given in hex representation.
pub type ChainId = u64;

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

// The execution status of the transaction
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "execution_status", content = "revert_reason")]
pub enum ExecutionStatus {
    #[serde(rename = "SUCCEEDED")]
    Successful,
    #[serde(rename = "REVERTED")]
    Reverted(String),
}

/// The resources consumed by the VM
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Default)]
pub struct ComputationResources {
    /// the number of BITWISE builtin instances
    #[serde(default)]
    pub bitwise_builtin_applications: Option<u64>,
    /// the number of EC_OP builtin instances
    #[serde(default)]
    pub ec_op_builtin_applications: Option<u64>,
    /// the number of ECDSA builtin instances
    #[serde(default)]
    pub ecdsa_builtin_applications: Option<u64>,
    /// The number of KECCAK builtin instances
    #[serde(default)]
    pub keccak_builtin_applications: Option<u64>,
    /// The number of unused memory cells (each cell is roughly equivalent to a step)
    #[serde(default)]
    pub memory_holes: Option<u64>,
    /// The number of Pedersen builtin instances
    #[serde(default)]
    pub pedersen_builtin_applications: Option<u64>,
    /// The number of Poseidon builtin instances
    #[serde(default)]
    pub poseidon_builtin_applications: Option<u64>,
    /// The number of RANGE_CHECK builtin instances
    #[serde(default)]
    pub range_check_builtin_applications: Option<u64>,
    /// The number of accesses to the segment arena
    #[serde(default)]
    pub segment_arena_builtin: Option<u64>,
    /// The number of Cairo steps used
    pub steps: u64,
}

pub type ContractAbi = Vec<ContractAbiEntry>;

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ContractAbiEntry {
    Function(FunctionAbiEntry),
    Event(EventAbiEntry),
    Struct(StructAbiEntry),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ContractClass {
    /// The class ABI, as supplied by the user declaring the class
    #[serde(default)]
    pub abi: Option<String>,
    /// The version of the contract class object. Currently, the Starknet OS supports version 0.1.0
    pub contract_class_version: String,
    pub entry_points_by_type: EntryPointsByType,
    /// The list of Sierra instructions of which the program consists
    pub sierra_program: Vec<Felt>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EntryPointsByType {
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<SierraEntryPoint>,
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<SierraEntryPoint>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<SierraEntryPoint>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ContractStorageDiffItem {
    /// The contract address for which the storage changed
    pub address: Felt,
    /// The changes in the storage of the contract
    pub storage_entries: Vec<KeyValuePair>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct KeyValuePair {
    /// The key of the changed value
    pub key: Felt,
    /// The new value applied to the given address
    pub value: Felt,
}

/// Specifies a storage domain in Starknet. Each domain has different gurantess regarding availability
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum DaMode {
    L1,
    L2,
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
pub struct DeclareTxnV0 {
    /// The hash of the declared class
    pub class_hash: Felt,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeclareTxnV1 {
    /// The hash of the declared class
    pub class_hash: Felt,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub nonce: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeclareTxnV2 {
    /// The hash of the declared class
    pub class_hash: Felt,
    /// The hash of the Cairo assembly resulting from the Sierra compilation
    pub compiled_class_hash: Felt,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub nonce: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployedContractItem {
    /// The address of the contract
    pub address: Felt,
    /// The hash of the contract code
    pub class_hash: Felt,
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
pub struct DeployAccountTxnV1 {
    /// The hash of the deployed contract's class
    pub class_hash: Felt,
    /// The parameters passed to the constructor
    pub constructor_calldata: Vec<Felt>,
    /// The salt for the address of the deployed contract
    pub contract_address_salt: Felt,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub nonce: Felt,
    pub signature: Signature,
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
pub struct DeployTxn {
    /// The hash of the deployed contract's class
    pub class_hash: Felt,
    /// The parameters passed to the constructor
    pub constructor_calldata: Vec<Felt>,
    /// The salt for the address of the deployed contract
    pub contract_address_salt: Felt,
    /// Version of the transaction scheme
    pub version: Felt,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeployTxnReceipt {
    #[serde(flatten)]
    pub common_receipt_properties: CommonReceiptProperties,
    /// The address of the deployed contract
    pub contract_address: Felt,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeprecatedCairoEntryPoint {
    /// The offset of the entry point in the program
    #[serde(with = "NumAsHex")]
    pub offset: u64,
    /// A unique identifier of the entry point (function) in the program
    pub selector: Felt,
}

/// The definition of a StarkNet contract class
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeprecatedContractClass {
    #[serde(default)]
    pub abi: Option<ContractAbi>,
    pub entry_points_by_type: DeprecatedEntryPointsByType,
    /// A base64 representation of the compressed program code
    pub program: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeprecatedEntryPointsByType {
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<DeprecatedCairoEntryPoint>,
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<DeprecatedCairoEntryPoint>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<DeprecatedCairoEntryPoint>,
}

/// Event information decorated with metadata on where it was emitted / An event emitted as a result of transaction execution
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
}

/// an ethereum address represented as 40 hex digits
pub type EthAddress = String;

/// A StarkNet event
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub from_address: Address,
    #[serde(flatten)]
    pub event_content: EventContent,
}

/// The content of an event
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EventContent {
    pub data: Vec<Felt>,
    pub keys: Vec<Felt>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EventsChunk {
    /// Use this token in a subsequent query to obtain the next page. Should not appear if there are no more pages.
    #[serde(default)]
    pub continuation_token: Option<String>,
    pub events: Vec<EmittedEvent>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EventAbiEntry {
    pub data: Vec<TypedParameter>,
    pub keys: Vec<TypedParameter>,
    /// The event name
    pub name: String,
    #[serde(rename = "type")]
    pub ty: EventAbiType,
}

pub type EventAbiType = String;

/// the resources consumed by the transaction, includes both computation and data
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResources {
    /// the number of BITWISE builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitwise_builtin_applications: Option<u64>,
    /// the number of EC_OP builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ec_op_builtin_applications: Option<u64>,
    /// the number of ECDSA builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ecdsa_builtin_applications: Option<u64>,
    /// The number of KECCAK builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keccak_builtin_applications: Option<u64>,
    /// The number of unused memory cells (each cell is roughly equivalent to a step)
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_holes: Option<u64>,
    /// The number of Pedersen builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pedersen_builtin_applications: Option<u64>,
    /// The number of Poseidon builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poseidon_builtin_applications: Option<u64>,
    /// The number of RANGE_CHECK builtin instances
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_check_builtin_applications: Option<u64>,
    /// The number of accesses to the segment arena
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_arena_builtin: Option<u64>,
    /// The number of Cairo steps used
    pub steps: u64,
    pub data_availability: DataAvailability,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DataAvailability {
    /// the data gas consumed by this transaction's data, 0 if it uses gas for DA
    pub l1_data_gas: u128,
    /// the gas consumed by this transaction's data, 0 if it uses data gas for DA
    pub l1_gas: u128,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeeEstimate {
    /// The Ethereum data gas consumption of the transaction
    pub data_gas_consumed: Felt,
    /// The data gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    pub data_gas_price: Felt,
    /// The Ethereum gas consumption of the transaction
    pub gas_consumed: Felt,
    /// The gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    pub gas_price: Felt,
    /// The estimated fee for the transaction (in wei or fri, depending on the tx version), equals to gas_consumed*gas_price + data_gas_consumed*data_gas_price
    pub overall_fee: Felt,
    /// units in which the fee is given
    pub unit: PriceUnit,
}

/// fee payment info as it appears in receipts
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeePayment {
    /// amount paid
    pub amount: Felt,
    /// units in which the fee is given
    pub unit: PriceUnit,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FunctionAbiEntry {
    pub inputs: Vec<TypedParameter>,
    /// The function name
    pub name: String,
    pub outputs: Vec<TypedParameter>,
    #[serde(default)]
    #[serde(rename = "stateMutability")]
    pub state_mutability: Option<FunctionStateMutability>,
    #[serde(rename = "type")]
    pub ty: FunctionAbiType,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum FunctionAbiType {
    #[serde(rename = "constructor")]
    Constructor,
    #[serde(rename = "function")]
    Function,
    #[serde(rename = "l1_handler")]
    L1Handler,
}

/// Function call information
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    /// The parameters passed to the function
    pub calldata: Calldata,
    pub contract_address: Address,
    pub entry_point_selector: Felt,
}

pub type FunctionStateMutability = String;

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

/// Invokes a specific function in the desired contract (not necessarily an account)
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct InvokeTxnV0 {
    /// The parameters passed to the function
    pub calldata: Calldata,
    pub contract_address: Address,
    pub entry_point_selector: Felt,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub signature: Signature,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct InvokeTxnV1 {
    /// The data expected by the account's `execute` function (in most usecases, this includes the called contract address and a function selector)
    pub calldata: Calldata,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Address,
    pub signature: Signature,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct L1HandlerTxn {
    /// The L1->L2 message nonce field of the SN Core L1 contract at the time the transaction was sent
    #[serde(with = "NumAsHex")]
    pub nonce: u64,
    /// Version of the transaction scheme
    pub version: String, /* 0x0 */
    #[serde(flatten)]
    pub function_call: FunctionCall,
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
pub struct MsgFromL1 {
    /// The selector of the l1_handler in invoke in the target contract
    pub entry_point_selector: Felt,
    /// The address of the L1 contract sending the message
    pub from_address: EthAddress,
    /// The payload of the message
    pub payload: Vec<Felt>,
    /// The target L2 address the message is sent to
    pub to_address: Address,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MsgToL1 {
    /// The address of the L2 contract sending the message
    pub from_address: Felt,
    /// The payload of the message
    pub payload: Vec<Felt>,
    /// The target L1 address the message is sent to
    pub to_address: Felt,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingBlockHeader {
    /// specifies whether the data of this block is published via blob data or calldata
    pub l1_da_mode: L1DaMode,
    /// The price of l1 data gas in the block
    pub l1_data_gas_price: ResourcePrice,
    /// The price of l1 gas in the block
    pub l1_gas_price: ResourcePrice,
    /// The hash of this block's parent
    pub parent_hash: BlockHash,
    /// The StarkNet identity of the sequencer submitting this block
    pub sequencer_address: Felt,
    /// Semver of the current Starknet protocol
    pub starknet_version: String,
    /// The time in which the block was created, encoded in Unix time
    pub timestamp: u64,
}

/// specifies whether the data of this block is published via blob data or calldata
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum L1DaMode {
    #[serde(rename = "BLOB")]
    Blob,
    #[serde(rename = "CALLDATA")]
    Calldata,
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

/// Pending state update
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingStateUpdate {
    /// The previous global state root
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum PriceUnit {
    #[serde(rename = "FRI")]
    Fri,
    #[serde(rename = "WEI")]
    Wei,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Default)]
pub struct ResourceBounds {
    /// the max amount of the resource that can be used in the tx
    #[serde(with = "NumAsHex")]
    pub max_amount: u64,
    /// the max price per unit of this resource for this tx
    #[serde(with = "NumAsHex")]
    pub max_price_per_unit: u128,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ResourceBoundsMapping {
    /// The max amount and max price per unit of L1 gas used in this tx
    pub l1_gas: ResourceBounds,
    /// The max amount and max price per unit of L2 gas used in this tx
    pub l2_gas: ResourceBounds,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ResourcePrice {
    /// the price of one unit of the given resource, denominated in fri (10^-18 strk)
    pub price_in_fri: Felt,
    /// the price of one unit of the given resource, denominated in wei
    pub price_in_wei: Felt,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SierraEntryPoint {
    /// The index of the function in the program
    pub function_idx: u64,
    /// A unique identifier of the entry point (function) in the program
    pub selector: Felt,
}

/// A transaction signature
pub type Signature = Arc<Vec<Felt>>;

/// A transaction calldata
pub type Calldata = Arc<Vec<Felt>>;

/// Flags that indicate how to simulate a given transaction. By default, the sequencer behavior is replicated locally
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum SimulationFlagForEstimateFee {
    #[serde(rename = "SKIP_VALIDATE")]
    SkipValidate,
}

/// The change in state applied in this block, given as a mapping of addresses to the new values and/or new contracts
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StateDiff {
    pub declared_classes: Vec<NewClasses>,
    pub deployed_contracts: Vec<DeployedContractItem>,
    pub deprecated_declared_classes: Vec<Felt>,
    pub nonces: Vec<NonceUpdate>,
    pub replaced_classes: Vec<ReplacedClass>,
    pub storage_diffs: Vec<ContractStorageDiffItem>,
}

/// The declared class hash and compiled class hash
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NewClasses {
    /// The hash of the declared class
    pub class_hash: Felt,
    /// The Cairo assembly hash corresponding to the declared class
    pub compiled_class_hash: Felt,
}

/// The updated nonce per contract address
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NonceUpdate {
    /// The address of the contract
    pub contract_address: Address,
    /// The nonce for the given address at the end of the block
    pub nonce: Felt,
}

/// The list of contracts whose class was replaced
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ReplacedClass {
    /// The new class hash
    pub class_hash: Felt,
    /// The address of the contract whose class was replaced
    pub contract_address: Address,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StateUpdate {
    pub block_hash: BlockHash,
    /// The new global state root
    pub new_root: Felt,
    /// The previous global state root
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

/// A storage key. Represented as up to 62 hex digits, 3 bits, and 5 leading zeroes.
pub type StorageKey = String;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StructAbiEntry {
    pub members: Vec<StructMember>,
    /// The struct name
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub ty: StructAbiType,
}

pub type StructAbiType = String;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StructMember {
    #[serde(flatten)]
    pub typed_parameter: TypedParameter,
    /// offset of this property within the struct
    pub offset: u64,
}

/// An object describing the node synchronization status
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SyncStatus {
    /// The hash of the current block being synchronized
    pub current_block_hash: BlockHash,
    /// The number (height) of the current block being synchronized
    pub current_block_num: BlockNumber,
    /// The hash of the estimated highest block to be synchronized
    pub highest_block_hash: BlockHash,
    /// The number (height) of the estimated highest block to be synchronized
    pub highest_block_num: BlockNumber,
    /// The hash of the block from which the sync started
    pub starting_block_hash: BlockHash,
    /// The number (height) of the block from which the sync started
    pub starting_block_num: BlockNumber,
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

/// The execution status of the transaction
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum TxnExecutionStatus {
    #[serde(rename = "REVERTED")]
    Reverted,
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
}

/// The finality status of the transaction
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug, Copy)]
pub enum TxnFinalityStatus {
    #[serde(rename = "ACCEPTED_ON_L1")]
    L1,
    #[serde(rename = "ACCEPTED_ON_L2")]
    L2,
}

/// The transaction hash, as assigned in StarkNet
pub type TxnHash = Felt;

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

/// The finality status of the transaction, including the case the txn is still in the mempool or failed validation during the block construction phase
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum TxnStatus {
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
    #[serde(rename = "RECEIVED")]
    Received,
    #[serde(rename = "REJECTED")]
    Rejected,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TypedParameter {
    /// The parameter's name
    pub name: String,
    /// The parameter's type
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockHashAndNumber {
    pub block_hash: BlockHash,
    pub block_number: BlockNumber,
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

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MaybeDeprecatedContractClass {
    Deprecated(DeprecatedContractClass),
    ContractClass(ContractClass),
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
pub enum MaybePendingStateUpdate {
    Block(StateUpdate),
    Pending(PendingStateUpdate),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TxnFinalityAndExecutionStatus {
    #[serde(default)]
    pub execution_status: Option<TxnExecutionStatus>,
    pub finality_status: TxnStatus,
}

/// Parameters of the `starknet_specVersion` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecVersionParams {}

impl Serialize for SpecVersionParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for SpecVersionParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SpecVersionParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_specVersion`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(1, &"expected 0 parameters"));
                }

                Ok(SpecVersionParams {})
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {}

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(SpecVersionParams {})
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getBlockWithTxHashes` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetBlockWithTxHashesParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for GetBlockWithTxHashesParams {
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

impl<'de> Deserialize<'de> for GetBlockWithTxHashesParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetBlockWithTxHashesParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getBlockWithTxHashes`")
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

                Ok(GetBlockWithTxHashesParams { block_id })
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

                Ok(GetBlockWithTxHashesParams { block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getBlockWithTxs` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetBlockWithTxsParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for GetBlockWithTxsParams {
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

impl<'de> Deserialize<'de> for GetBlockWithTxsParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetBlockWithTxsParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getBlockWithTxs`")
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

                Ok(GetBlockWithTxsParams { block_id })
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

                Ok(GetBlockWithTxsParams { block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getBlockWithReceipts` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetBlockWithReceiptsParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for GetBlockWithReceiptsParams {
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

impl<'de> Deserialize<'de> for GetBlockWithReceiptsParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetBlockWithReceiptsParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getBlockWithReceipts`")
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

                Ok(GetBlockWithReceiptsParams { block_id })
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

                Ok(GetBlockWithReceiptsParams { block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getStateUpdate` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetStateUpdateParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for GetStateUpdateParams {
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

impl<'de> Deserialize<'de> for GetStateUpdateParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetStateUpdateParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getStateUpdate`")
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

                Ok(GetStateUpdateParams { block_id })
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

                Ok(GetStateUpdateParams { block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getStorageAt` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetStorageAtParams {
    /// The address of the contract to read from
    pub contract_address: Address,
    /// The key to the storage value for the given contract
    pub key: StorageKey,
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for GetStorageAtParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("contract_address", &self.contract_address)?;
        map.serialize_entry("key", &self.key)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetStorageAtParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetStorageAtParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getStorageAt`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let contract_address: Address =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 3 parameters"))?;
                let key: StorageKey =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 3 parameters"))?;
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(3, &"expected 3 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(4, &"expected 3 parameters"));
                }

                Ok(GetStorageAtParams { contract_address, key, block_id })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    contract_address: Address,
                    key: StorageKey,
                    block_id: BlockId,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetStorageAtParams {
                    contract_address: helper.contract_address,
                    key: helper.key,
                    block_id: helper.block_id,
                })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getTransactionStatus` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetTransactionStatusParams {
    /// The hash of the requested transaction
    pub transaction_hash: TxnHash,
}

impl Serialize for GetTransactionStatusParams {
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

impl<'de> Deserialize<'de> for GetTransactionStatusParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetTransactionStatusParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getTransactionStatus`")
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

                Ok(GetTransactionStatusParams { transaction_hash: helper.transaction_hash })
            }
        }

        deserializer.deserialize_map(Visitor)
    }
}

/// Parameters of the `starknet_getTransactionByHash` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetTransactionByHashParams {
    /// The hash of the requested transaction
    pub transaction_hash: TxnHash,
}

impl Serialize for GetTransactionByHashParams {
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

impl<'de> Deserialize<'de> for GetTransactionByHashParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetTransactionByHashParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getTransactionByHash`")
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

                Ok(GetTransactionByHashParams { transaction_hash: helper.transaction_hash })
            }
        }

        deserializer.deserialize_map(Visitor)
    }
}

/// Parameters of the `starknet_getTransactionByBlockIdAndIndex` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetTransactionByBlockIdAndIndexParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
    /// The index in the block to search for the transaction
    pub index: u64,
}

impl Serialize for GetTransactionByBlockIdAndIndexParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.serialize_entry("index", &self.index)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetTransactionByBlockIdAndIndexParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetTransactionByBlockIdAndIndexParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getTransactionByBlockIdAndIndex`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let index: u64 =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(GetTransactionByBlockIdAndIndexParams { block_id, index })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                    index: u64,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetTransactionByBlockIdAndIndexParams { block_id: helper.block_id, index: helper.index })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getTransactionReceipt` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetTransactionReceiptParams {
    /// The hash of the requested transaction
    pub transaction_hash: TxnHash,
}

impl Serialize for GetTransactionReceiptParams {
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

impl<'de> Deserialize<'de> for GetTransactionReceiptParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetTransactionReceiptParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getTransactionReceipt`")
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

                Ok(GetTransactionReceiptParams { transaction_hash: helper.transaction_hash })
            }
        }

        deserializer.deserialize_map(Visitor)
    }
}

/// Parameters of the `starknet_getClass` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetClassParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
    /// The hash of the requested contract class
    pub class_hash: Felt,
}

impl Serialize for GetClassParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.serialize_entry("class_hash", &self.class_hash)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetClassParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetClassParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getClass`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let class_hash: Felt =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(GetClassParams { block_id, class_hash })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                    class_hash: Felt,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetClassParams { block_id: helper.block_id, class_hash: helper.class_hash })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getClassHashAt` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetClassHashAtParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
    /// The address of the contract whose class hash will be returned
    pub contract_address: Address,
}

impl Serialize for GetClassHashAtParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.serialize_entry("contract_address", &self.contract_address)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetClassHashAtParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetClassHashAtParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getClassHashAt`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let contract_address: Address =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(GetClassHashAtParams { block_id, contract_address })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                    contract_address: Address,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetClassHashAtParams { block_id: helper.block_id, contract_address: helper.contract_address })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getClassAt` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetClassAtParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
    /// The address of the contract whose class definition will be returned
    pub contract_address: Address,
}

impl Serialize for GetClassAtParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.serialize_entry("contract_address", &self.contract_address)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetClassAtParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetClassAtParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getClassAt`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let contract_address: Address =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(GetClassAtParams { block_id, contract_address })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                    contract_address: Address,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetClassAtParams { block_id: helper.block_id, contract_address: helper.contract_address })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getBlockTransactionCount` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetBlockTransactionCountParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
}

impl Serialize for GetBlockTransactionCountParams {
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

impl<'de> Deserialize<'de> for GetBlockTransactionCountParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetBlockTransactionCountParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getBlockTransactionCount`")
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

                Ok(GetBlockTransactionCountParams { block_id })
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

                Ok(GetBlockTransactionCountParams { block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_call` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallParams {
    /// The details of the function call
    pub request: FunctionCall,
    /// The hash of the requested block, or number (height) of the requested block, or a block tag, for the block referencing the state or call the transaction on.
    pub block_id: BlockId,
}

impl Serialize for CallParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("request", &self.request)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for CallParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CallParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_call`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let request: FunctionCall =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(CallParams { request, block_id })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    request: FunctionCall,
                    block_id: BlockId,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(CallParams { request: helper.request, block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_estimateFee` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EstimateFeeParams {
    /// The transaction to estimate
    pub request: Vec<BroadcastedTxn>,
    /// describes what parts of the transaction should be executed
    pub simulation_flags: Vec<SimulationFlagForEstimateFee>,
    /// The hash of the requested block, or number (height) of the requested block, or a block tag, for the block referencing the state or call the transaction on.
    pub block_id: BlockId,
}

impl Serialize for EstimateFeeParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("request", &self.request)?;
        map.serialize_entry("simulation_flags", &self.simulation_flags)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for EstimateFeeParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = EstimateFeeParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_estimateFee`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let request: Vec<BroadcastedTxn> =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 3 parameters"))?;
                let simulation_flags: Vec<SimulationFlagForEstimateFee> =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 3 parameters"))?;
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(3, &"expected 3 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(4, &"expected 3 parameters"));
                }

                Ok(EstimateFeeParams { request, simulation_flags, block_id })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    request: Vec<BroadcastedTxn>,
                    simulation_flags: Vec<SimulationFlagForEstimateFee>,
                    block_id: BlockId,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(EstimateFeeParams {
                    request: helper.request,
                    simulation_flags: helper.simulation_flags,
                    block_id: helper.block_id,
                })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_estimateMessageFee` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EstimateMessageFeeParams {
    /// the message's parameters
    pub message: MsgFromL1,
    /// The hash of the requested block, or number (height) of the requested block, or a block tag, for the block referencing the state or call the transaction on.
    pub block_id: BlockId,
}

impl Serialize for EstimateMessageFeeParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("message", &self.message)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for EstimateMessageFeeParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = EstimateMessageFeeParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_estimateMessageFee`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let message: MsgFromL1 =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(EstimateMessageFeeParams { message, block_id })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    message: MsgFromL1,
                    block_id: BlockId,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(EstimateMessageFeeParams { message: helper.message, block_id: helper.block_id })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_blockNumber` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockNumberParams {}

impl Serialize for BlockNumberParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for BlockNumberParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = BlockNumberParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_blockNumber`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(1, &"expected 0 parameters"));
                }

                Ok(BlockNumberParams {})
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {}

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(BlockNumberParams {})
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_blockHashAndNumber` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockHashAndNumberParams {}

impl Serialize for BlockHashAndNumberParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for BlockHashAndNumberParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = BlockHashAndNumberParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_blockHashAndNumber`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(1, &"expected 0 parameters"));
                }

                Ok(BlockHashAndNumberParams {})
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {}

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(BlockHashAndNumberParams {})
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_chainId` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainIdParams {}

impl Serialize for ChainIdParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for ChainIdParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ChainIdParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_chainId`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(1, &"expected 0 parameters"));
                }

                Ok(ChainIdParams {})
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {}

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(ChainIdParams {})
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_syncing` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncingParams {}

impl Serialize for SyncingParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for SyncingParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SyncingParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_syncing`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(1, &"expected 0 parameters"));
                }

                Ok(SyncingParams {})
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {}

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(SyncingParams {})
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getEvents` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetEventsParams {
    /// The conditions used to filter the returned events
    pub filter: EventFilterWithPageRequest,
}

impl Serialize for GetEventsParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("filter", &self.filter)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetEventsParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetEventsParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getEvents`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let filter: EventFilterWithPageRequest =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 1 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &"expected 1 parameters"));
                }

                Ok(GetEventsParams { filter })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    filter: EventFilterWithPageRequest,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetEventsParams { filter: helper.filter })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_getNonce` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetNonceParams {
    /// The hash of the requested block, or number (height) of the requested block, or a block tag
    pub block_id: BlockId,
    /// The address of the contract whose nonce we're seeking
    pub contract_address: Address,
}

impl Serialize for GetNonceParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("block_id", &self.block_id)?;
        map.serialize_entry("contract_address", &self.contract_address)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for GetNonceParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = GetNonceParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_getNonce`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let block_id: BlockId =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 2 parameters"))?;
                let contract_address: Address =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(2, &"expected 2 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &"expected 2 parameters"));
                }

                Ok(GetNonceParams { block_id, contract_address })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    block_id: BlockId,
                    contract_address: Address,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(GetNonceParams { block_id: helper.block_id, contract_address: helper.contract_address })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}
