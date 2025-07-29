use crate::v0_7_1::{
    BlockHash, BlockNumber, BlockStatus, BroadcastedDeclareTxnV3, DeployAccountTxnV3, InvokeTxnV3, L1DaMode, PriceUnit,
    ResourcePrice, TransactionAndReceipt, TxnExecutionStatus, TxnHash, TxnStatus, TxnWithHash,
};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

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
    pub transactions: Vec<TransactionAndReceipt>,
    pub status: BlockStatus,
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The block object
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithTxs {
    pub transactions: Vec<TxnWithHash>,
    pub status: BlockStatus,
    #[serde(flatten)]
    pub block_header: BlockHeader,
}

/// The block object
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlockWithTxHashes {
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
    pub transactions: Vec<TransactionAndReceipt>,
    #[serde(flatten)]
    pub pending_block_header: PendingBlockHeader,
}

/// TThe dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingBlockWithTxs {
    pub transactions: Vec<TxnWithHash>,
    #[serde(flatten)]
    pub pending_block_header: PendingBlockHeader,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TxnStatusResult {
    pub finality_status: TxnStatus,
    #[serde(default)]
    pub execution_status: Option<TxnExecutionStatus>,
    #[serde(default)]
    pub failure_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FeeEstimate {
    /// The Ethereum gas consumption of the transaction, charged for L1->L2 messages and, depending on the block's DA_MODE, state diffs
    pub l1_gas_consumed: u64,
    /// The gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    pub l1_gas_price: u128,
    /// The L2 gas consumption of the transaction
    pub l2_gas_consumed: u64,
    /// The L2 gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    pub l2_gas_price: u128,
    /// The Ethereum data gas consumption of the transaction
    pub l1_data_gas_consumed: u64,
    /// The data gas price (in wei or fri, depending on the tx version) that was used in the cost estimation
    pub l1_data_gas_price: u128,
    /// The estimated fee for the transaction (in wei or fri, depending on the tx version), equals to l1_gas_consumed*l1_gas_price + l1_data_gas_consumed*l1_data_gas_price + l2_gas_consumed*l2_gas_price
    pub overall_fee: u128,
    /// Fee unit
    pub unit: PriceUnit,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PendingBlockWithTxHashes {
    /// The hashes of the transactions included in this block
    pub transactions: Vec<TxnHash>,
    #[serde(flatten)]
    pub pending_block_header: PendingBlockHeader,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedDeclareTxn {
    #[serde(rename = "0x3")]
    V3(BroadcastedDeclareTxnV3),

    /// Query-only broadcasted declare transaction.
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(BroadcastedDeclareTxnV3),
}

impl BroadcastedDeclareTxn {
    pub fn is_query(&self) -> bool {
        match self {
            BroadcastedDeclareTxn::QueryV3(_) => true,
            BroadcastedDeclareTxn::V3(_) => false,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedDeployAccountTxn {
    #[serde(rename = "0x3")]
    V3(DeployAccountTxnV3),

    /// Query-only broadcasted deploy account transaction.
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(DeployAccountTxnV3),
}

impl BroadcastedDeployAccountTxn {
    pub fn is_query(&self) -> bool {
        match self {
            BroadcastedDeployAccountTxn::QueryV3(_) => true,
            BroadcastedDeployAccountTxn::V3(_) => false,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedInvokeTxn {
    #[serde(rename = "0x3")]
    V3(InvokeTxnV3),

    /// Query-only broadcasted invoke transaction.
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(InvokeTxnV3),
}

impl BroadcastedInvokeTxn {
    pub fn is_query(&self) -> bool {
        match self {
            BroadcastedInvokeTxn::QueryV3(_) => true,
            BroadcastedInvokeTxn::V3(_) => false,
        }
    }
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

/// a node_hash -> node mapping of all the nodes in the union of the paths between the requested leaves and the root
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
    /// The nodes in the union of the paths from the contracts tree root to the requested leaves
    pub nodes: Vec<NodeHashToNodeMappingItem>,
    /// The nonce and class hash for each requested contract address, in the order in which they appear in the request. These values are needed to construct the associated leaf node
    pub contract_leaves_data: Vec<ContractLeavesDataItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalRoots {
    pub contracts_tree_root: Felt,
    pub classes_tree_root: Felt,
    /// the associated block hash (needed in case the caller used a block tag for the block_id parameter)
    pub block_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStorageProofResult {
    /// The requested storage proofs. Note that if a requested leaf has the default value, the path to it may end in an edge node whose path is not a prefix of the requested leaf, thus effectively proving non-membership
    pub classes_proof: Vec<NodeHashToNodeMappingItem>,
    ///The nodes in the union of the paths from the contracts tree root to the requested leaves
    pub contracts_proof: ContractsProof,
    pub contracts_storage_proofs: Vec<Vec<NodeHashToNodeMappingItem>>,
    pub global_roots: GlobalRoots,
}
