//! Step 1. pre-validate: [`UnverifiedFullBlock`] ====[`crate::pre_validate`]===> [`PreValidatedBlock`]
//! Step 2. verify_apply: [`PreValidatedBlock`] ====[`crate::verify_apply`]===> [`BlockImportResult`]

use dp_block::{
    header::{GasPrices, L1DataAvailabilityMode},
    Header,
};
use dp_chain_config::StarknetVersion;
use dp_class::{ContractClass, ConvertedClass};
use dp_receipt::TransactionReceipt;
use dp_state_update::StateDiff;
use dp_transactions::Transaction;
use starknet_api::core::ChainId;
use starknet_core::types::Felt;

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct UnverifiedHeader {
    /// The hash of this block’s parent. When set to None, it will be deduced from the latest block in storage.
    pub parent_block_hash: Option<Felt>,
    /// The Starknet address of the sequencer that created this block.
    pub sequencer_address: Felt,
    /// The time the sequencer created this block before executing transactions
    pub block_timestamp: u64,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub l1_gas_price: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Validation {
    /// Use the transaction hashes from the transaction receipts instead of computing them.
    pub trust_transaction_hashes: bool,
    pub chain_id: ChainId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredClass {
    pub class_hash: Felt,
    pub contract_class: ContractClass,
    pub compiled_class_hash: Felt,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct UnverifiedCommitments {
    pub transaction_count: Option<u64>,
    pub transaction_commitment: Option<Felt>,
    pub event_count: Option<u64>,
    pub event_commitment: Option<Felt>,
    pub state_diff_length: Option<u64>,
    pub state_diff_commitment: Option<Felt>,
    pub receipt_commitment: Option<Felt>,
    /// Global state root
    pub global_state_root: Option<Felt>,
    /// Expected block hash
    pub block_hash: Option<Felt>,
}

/// An unverified pending full block as input for the block import pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnverifiedPendingFullBlock {
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
}

/// An unverified full block as input for the block import pipeline.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct UnverifiedFullBlock {
    /// When set to None, it will be deduced from the latest block in storage.
    pub unverified_block_number: Option<u64>,
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
    pub commitments: UnverifiedCommitments,
}

// Pre-validate outputs.

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ValidatedCommitments {
    pub transaction_count: u64,
    pub transaction_commitment: Felt,
    pub event_count: u64,
    pub event_commitment: Felt,
    pub state_diff_length: u64,
    pub state_diff_commitment: Felt,
    pub receipt_commitment: Felt,
}

/// Output of the [`crate::pre_validate`] step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreValidatedBlock {
    pub header: UnverifiedHeader,
    pub transactions: Vec<Transaction>,
    pub state_diff: StateDiff,
    pub receipts: Vec<TransactionReceipt>,
    pub commitments: ValidatedCommitments,
    pub converted_classes: Vec<ConvertedClass>,

    pub unverified_global_state_root: Option<Felt>,
    pub unverified_block_hash: Option<Felt>,
    pub unverified_block_number: Option<u64>,
}

/// Output of the [`crate::pre_validate`] step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreValidatedPendingBlock {
    pub header: UnverifiedHeader,
    pub transactions: Vec<Transaction>,
    pub state_diff: StateDiff,
    pub receipts: Vec<TransactionReceipt>,
    pub converted_classes: Vec<ConvertedClass>,
}

// Verify-apply output.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockImportResult {
    pub header: Header,
    pub block_hash: Felt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingBlockImportResult {}
