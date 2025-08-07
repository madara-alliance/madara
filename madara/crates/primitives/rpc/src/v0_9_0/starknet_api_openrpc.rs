use super::{BlockHash, BlockNumber, L1DaMode, ResourcePrice, TransactionAndReceipt, TxnHash, TxnReceipt, TxnWithHash};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

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

/// The status of the block
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum BlockStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
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
    pub pending_block_header: PreConfirmedBlockHeader,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithTxs {
    /// The transactions in this block
    pub transactions: Vec<TxnWithHash>,
    #[serde(flatten)]
    pub pending_block_header: PreConfirmedBlockHeader,
}

/// The dynamic block being constructed by the sequencer. Note that this object will be deprecated upon decentralization.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithTxHashes {
    /// The hashes of the transactions included in this block
    pub transactions: Vec<TxnHash>,
    #[serde(flatten)]
    pub pending_block_header: PreConfirmedBlockHeader,
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
#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum TxnFinalityStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    L2,
    #[serde(rename = "ACCEPTED_ON_L1")]
    L1,
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

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum PriceUnitWei {
    #[serde(rename = "WEI")]
    Wei,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum PriceUnitFri {
    #[serde(rename = "FRI")]
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
