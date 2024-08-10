//! Block verification/import pipeline.
//!
//! ## Architecture
//!
//! Block validation works in 4 steps:
//!
//! ### Step 0: Fetching.
//!
//! Step 0 is handled by the consumers of this crate. It can use FGW / peer-to-peer / sync from rpc or any other fetching mechanism.
//! The fetching process is expected to be parallel using [`tokio`] and to put its [`UnverifiedFullBlock`]s in the input channel.
//!
//! ### Step 1: Block pre-validate.
//!
//! This step is parallelized over [`PRE_VALIDATE_PIPELINE_LEN`] blocks. It also uses [`rayon`] for intra-block parallelization.
//! This step checks all of the commitments of a block except the global state root and the block hash .
//! This is also where classes are compiled.
//! This does not read nor update the database.
//!
//! ### Step 2: Apply block to global tries.
//!
//! This step is necessarily sequencial over blocks, but parallelization is done internally using [`rayon`].
//! This is where the final `state_root` and `block_hash` are computed.
//!
//! ### Step 2.5: Store block and classes.
//!
//! This step is also sequencial but ises internal parallelization using [`rayon`].
//!
//! ## Error handling
//!
//! When using p2p, validating a block could fail but that shouldn't close the whole app. This requires a retrying mechanism
//! to get a different block from another peer, and you want to lower the peer score of the offending peer. The plumbery to
//! get that done is not supported yet but it has been incorporated in the design of this pipeline.
//! 
//! ## Pull/Push based
//! 
//! The block import pipeline [`BlockImportService::drive_pipeline`] is pull based, and it will concurrently call
//! [`BlockFetcher::fetch_block`] to pull new blocks into the pipeline. This is the fastest way of importing, and
//! it supports backpressure correctly. However, this is not fit every usecase: when the chain is fully synced,
//! consumers of this crate are expected to use the push-based [`BlockImportService::import_block`] to push new new blocks
//! when they appear on the network.
//! The pipeline also implements a polling mode when [`PipelineSettings::polling`] is not `None`: in this mode, once the
//! block pipeline has reached the tip of the blockchain, it will continue polling on an interval for more blocks. When
//! this is enabled, the pipeline task until gracefyl shutdown.
//! The polling mode is used to implement sync from feeder gateway / rpc where we can't have live updates.
//!
//! ## Future plans
//!
//! An optional sequencial step just before step 2 could be addded that executes the block to validate it: this will
//! be useful for tendermint validator nodes in the future, and it should also be useful to test-execute a whole blockchain
//! to check for errors.
//! A signature verification mode should be added to allow the skipping of block validation entirely if the block is signed.

use std::{borrow::Cow, sync::Arc, time::Duration};

use dc_db::{DeoxysBackend, DeoxysStorageError};
use dp_block::header::{GasPrices, L1DataAvailabilityMode};
use dp_chain_config::StarknetVersion;
use dp_class::ContractClass;
use dp_receipt::TransactionReceipt;
use dp_state_update::StateDiff;
use dp_transactions::Transaction;
use starknet_api::core::ChainId;
use starknet_core::types::Felt;

mod pipeline;
mod pre_validate;
mod verify_apply;

pub use pre_validate::*;
pub use verify_apply::*;

#[derive(Debug, thiserror::Error)]
pub enum BlockImportError {
    #[error("Transaction count and receipt count do not match: {receipts} receipts != {transactions} transactions")]
    TransactionEqualReceiptCount { receipts: usize, transactions: usize },

    #[error("Transaction hash mismatch for index #{index}: expected {expected:#x}, got {got:#x}")]
    TransactionHash { index: usize, got: Felt, expected: Felt },
    #[error("Transaction count mismatch: expected {expected}, got {got}")]
    TransactionCount { got: u64, expected: u64 },
    #[error("Transaction commitment mismatch: expected {expected:#x}, got {got:#x}")]
    TransactionCommitment { got: Felt, expected: Felt },

    #[error("Event count mismatch: expected {expected}, got {got}")]
    EventCount { got: u64, expected: u64 },
    #[error("Event commitment mismatch: expected {expected:#x}, got {got:#x}")]
    EventCommitment { got: Felt, expected: Felt },

    #[error("State diff length mismatch: expected {expected}, got {got}")]
    StateDiffLength { got: u64, expected: u64 },
    #[error("State diff commitment mismatch: expected {expected:#x}, got {got:#x}")]
    StateDiffCommitment { got: Felt, expected: Felt },

    #[error("Receipt commitment mismatch: expected {expected:#x}, got {got:#x}")]
    ReceiptCommitment { got: Felt, expected: Felt },

    #[error("Class hash mismatch: expected {expected:#x}, got {got:#x}")]
    ClassHash { got: Felt, expected: Felt },
    #[error("Compiled class hash mismatch for class hash {class_hash:#x}: expected {expected:#x}, got {got:#x}")]
    CompiledClassHash { class_hash: Felt, got: Felt, expected: Felt },
    #[error("Class with hash {class_hash:#x} failed to compile: {error}")]
    CompilationClassError { class_hash: Felt, error: String },

    #[error("Block order mismatch: database expects to import block #{expected}, trying to import #{got}")]
    LatestBlockN { expected: u64, got: u64 },
    #[error("Parent hash mismatch: expected {expected:#x}, got {got:#x}")]
    ParentHash { got: Felt, expected: Felt },
    #[error("Global state root mismatch: expected {expected:#x}, got {got:#x}")]
    GlobalStateRoot { got: Felt, expected: Felt },

    #[error("Internal database error while {context}: {error:#}")]
    InternalDb { context: Cow<'static, str>, error: DeoxysStorageError },
    #[error("Internal error: {0}")]
    Internal(Cow<'static, str>),
    #[error("Internal fetching error: {0}")]
    InternalFetching(anyhow::Error),
}

impl BlockImportError {
    /// Check this to see if this is an internal potentially unreconverable error. Useful in p2p
    /// for differenciating between when a peer sent an invalid block and internal db errors.
    pub fn is_internal(&self) -> bool {
        match self {
            BlockImportError::InternalDb { .. } => true,
            BlockImportError::Internal(_) => true,
            BlockImportError::InternalFetching(_) => true,
            _ => false,
        }
    }
}

/// Compute the pre-validation step in parallel over 10 blocks.
const PRE_VALIDATE_PIPELINE_LEN: usize = 10;

pub struct UnverifiedHeader {
    /// The hash of this block’s parent.
    pub parent_block_hash: Felt,
    /// The number (height) of this block.
    pub block_number: u64,
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

#[derive(Clone, Debug)]
pub struct Validation {
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
    /// Use the transaction hashes from the transaction receipts instead of computing them.
    pub trust_transaction_hashes: bool,

    pub chain_id: ChainId,
}

#[derive(Clone, Debug)]
pub struct DeclaredClass {
    pub class_hash: Felt,
    pub contract_class: ContractClass,
    pub compiled_class_hash: Felt,
}

/// An unverified full block as input for the block import pipeline.
pub struct UnverifiedFullBlock {
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
}

#[derive(thiserror::Error, Debug)]
pub enum BlockFetcherError {
    /// Block not found: this instructs the pipeline to stop parallel fetching and start block polling.
    #[error("Block not found")]
    BlockNotFound,
    #[error(transparent)]
    Custom(#[from] anyhow::Error),
}

pub struct ImportErrorResult {
    /// Set to `true` to shutdown block import.
    pub shutdown: bool,
}

#[async_trait::async_trait]
pub trait BlockFetcher: Send + Sync {
    async fn fetch_block(&self, block_n: u64) -> Result<UnverifiedFullBlock, BlockFetcherError>;
}

pub struct SyncPollingSettings {
    pub interval: Duration,
}

pub struct PipelineSettings {
    pub logs: bool,
    pub first_block: Option<u64>,
    pub n_blocks_to_sync: Option<u64>,
    pub end_at: Option<u64>,
    pub polling: Option<SyncPollingSettings>,
    /// Default: 10
    pub fetch_convert_stream_buffer: usize,
    /// Channel size between the fetch/convert task and apply/verify.
    /// Default: 10
    pub channel_size: usize
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            logs: false,
            first_block: None,
            n_blocks_to_sync: None,
            end_at: None,
            polling: Some(SyncPollingSettings { interval: Duration::from_millis(2000) }),
            fetch_convert_stream_buffer: 10,
            channel_size: 10,
        }
    }
}

pub struct BlockImportService {
    backend: Arc<DeoxysBackend>,
    verify_apply: VerifyApply,
}

impl BlockImportService {
    pub fn new(backend: Arc<DeoxysBackend>) -> Self {
        Self { verify_apply: VerifyApply::new(Arc::clone(&backend)), backend }
    }

    /// Import a single block.
    pub async fn import_block(
        &self,
        block: UnverifiedFullBlock,
        validation: &Validation,
    ) -> Result<(), BlockImportError> {
        let block = pre_validate(block, validation).await?;
        self.verify_apply.verify_apply(block, validation).await?;
        Ok(())
    }
}
