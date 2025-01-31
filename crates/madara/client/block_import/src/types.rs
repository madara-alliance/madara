//! Step 1. pre-validate: [`UnverifiedFullBlock`] ====[`crate::pre_validate`]===> [`PreValidatedBlock`]
//! Step 2. verify_apply: [`PreValidatedBlock`] ====[`crate::verify_apply`]===> [`BlockImportResult`]

use mp_block::{
    header::{BlockTimestamp, GasPrices, L1DataAvailabilityMode},
    Header,
};
use mp_chain_config::StarknetVersion;
use mp_class::{
    class_update::{ClassUpdate, LegacyClassUpdate, SierraClassUpdate},
    CompressedLegacyContractClass, ConvertedClass, FlattenedSierraClass,
};
use mp_receipt::TransactionReceipt;
use mp_state_update::StateDiff;
use mp_transactions::Transaction;
use serde::{Deserialize, Serialize};
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;

#[derive(Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct UnverifiedHeader {
    /// The hash of this block’s parent. When set to None, it will be deduced from the latest block in storage.
    pub parent_block_hash: Option<Felt>,
    /// The Starknet address of the sequencer that created this block.
    pub sequencer_address: Felt,
    /// The time the sequencer created this block before executing transactions
    pub block_timestamp: BlockTimestamp,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub l1_gas_price: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockValidationContext {
    /// Use the transaction hashes from the transaction receipts instead of computing them.
    pub trust_transaction_hashes: bool,
    /// Trust class hashes.
    pub trust_class_hashes: bool,
    /// Do not recomppute the trie commitments, trust them instead.
    /// If the global state root commitment is missing during import, this will error.
    /// This is only intended for full-node syncing without storing the global trie.
    pub trust_global_tries: bool,
    /// Ignore the order of the blocks to allow starting at some height.
    pub ignore_block_order: bool,
    /// The chain id of the current block.
    pub chain_id: ChainId,
}

impl BlockValidationContext {
    pub fn new(chain_id: ChainId) -> Self {
        Self {
            trust_transaction_hashes: false,
            trust_class_hashes: false,
            trust_global_tries: false,
            chain_id,
            ignore_block_order: false,
        }
    }
    pub fn trust_transaction_hashes(mut self, v: bool) -> Self {
        self.trust_transaction_hashes = v;
        self
    }
    pub fn trust_class_hashes(mut self, v: bool) -> Self {
        self.trust_class_hashes = v;
        self
    }
    pub fn trust_global_tries(mut self, v: bool) -> Self {
        self.trust_global_tries = v;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DeclaredClass {
    Legacy(LegacyDeclaredClass),
    Sierra(SierraDeclaredClass),
}

impl DeclaredClass {
    pub fn class_hash(&self) -> Felt {
        match self {
            DeclaredClass::Legacy(c) => c.class_hash,
            DeclaredClass::Sierra(c) => c.class_hash,
        }
    }
}

impl From<ClassUpdate> for DeclaredClass {
    fn from(value: ClassUpdate) -> Self {
        match value {
            ClassUpdate::Legacy(legacy) => DeclaredClass::Legacy(legacy.into()),
            ClassUpdate::Sierra(sierra) => DeclaredClass::Sierra(sierra.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LegacyDeclaredClass {
    pub class_hash: Felt,
    pub contract_class: CompressedLegacyContractClass,
}

impl From<LegacyClassUpdate> for LegacyDeclaredClass {
    fn from(value: LegacyClassUpdate) -> Self {
        Self { class_hash: value.class_hash, contract_class: value.contract_class }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SierraDeclaredClass {
    pub class_hash: Felt,
    pub contract_class: FlattenedSierraClass,
    pub compiled_class_hash: Felt,
}

impl From<SierraClassUpdate> for SierraDeclaredClass {
    fn from(value: SierraClassUpdate) -> Self {
        Self {
            class_hash: value.class_hash,
            contract_class: value.contract_class,
            compiled_class_hash: value.compiled_class_hash,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct UnverifiedPendingFullBlock {
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
}

/// An unverified full block as input for the block import pipeline.
#[derive(Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct UnverifiedFullBlock {
    /// When set to None, it will be deduced from the latest block in storage.
    pub unverified_block_number: Option<u64>,
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
    /// Classes that are already compiled and hashed.
    #[serde(skip)]
    pub trusted_converted_classes: Vec<ConvertedClass>,
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
