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
//! ## Future plans
//!
//! An optional sequencial step just before step 2 could be addded that executes the block to validate it: this will
//! be useful for tendermint validator nodes in the future, and it should also be useful to test-execute a whole blockchain
//! to check for errors.
//! A signature verification mode should be added to allow the skipping of block validation entirely if the block is signed.

use anyhow::Context;
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_metrics::MetricsRegistry;
use metrics::BlockMetrics;
use mp_class::{class_hash::ComputeClassHashError, compile::ClassCompilationError};
use starknet_core::types::Felt;
use std::{borrow::Cow, sync::Arc};

mod metrics;
mod pre_validate;
mod rayon;
pub mod tests;
mod types;
mod verify_apply;
pub use pre_validate::*;
pub use rayon::*;
pub use types::*;
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
    CompilationClassError { class_hash: Felt, error: ClassCompilationError },
    #[error("Failed to compute class hash {class_hash:#x}: {error}")]
    ComputeClassHash { class_hash: Felt, error: ComputeClassHashError },

    #[error("Block hash mismatch: expected {expected:#x}, got {got:#x}")]
    BlockHash { got: Felt, expected: Felt },

    #[error("Block order mismatch: database expects to import block #{expected}, trying to import #{got}. To import a block out of order, use the `ignore_block_order` flag.")]
    LatestBlockN { expected: u64, got: u64 },
    #[error("Parent hash mismatch: expected {expected:#x}, got {got:#x}")]
    ParentHash { got: Felt, expected: Felt },
    #[error("Global state root mismatch: expected {expected:#x}, got {got:#x}")]
    GlobalStateRoot { got: Felt, expected: Felt },

    /// Internal error, see [`BlockImportError::is_internal`].
    #[error("Internal database error while {context}: {error:#}")]
    InternalDb { context: Cow<'static, str>, error: MadaraStorageError },
    /// Internal error, see [`BlockImportError::is_internal`].
    #[error("Internal error: {0}")]
    Internal(Cow<'static, str>),
}

impl BlockImportError {
    /// Unrecoverable errors.
    pub fn is_internal(&self) -> bool {
        matches!(self, BlockImportError::InternalDb { .. } | BlockImportError::Internal(_))
    }
}
pub struct BlockImporter {
    pool: Arc<RayonPool>,
    backend: Arc<MadaraBackend>,
    verify_apply: VerifyApply,
    metrics: BlockMetrics,
    always_force_flush: bool,
}

impl BlockImporter {
    /// The starting block is used for metrics. Setting it to None means it will look at the database latest block number.
    pub fn new(
        backend: Arc<MadaraBackend>,
        metrics_registry: &MetricsRegistry,
        starting_block: Option<u64>,
        always_force_flush: bool,
    ) -> anyhow::Result<Self> {
        let pool = Arc::new(RayonPool::new());
        let starting_block = if let Some(n) = starting_block {
            n
        } else {
            backend
                .get_latest_block_n()
                .context("Getting latest block in database")?
                .map(|b| /* next block */ b + 1)
                .unwrap_or(0 /* genesis */)
        };

        Ok(Self {
            verify_apply: VerifyApply::new(Arc::clone(&backend), Arc::clone(&pool)),
            pool,
            metrics: BlockMetrics::register(starting_block, metrics_registry)
                .context("Registering metrics for block import")?,
            backend,
            always_force_flush,
        })
    }

    /// Perform [`BlockImporter::pre_validate`] followed by [`BlockImporter::verify_apply`] to import a block.
    pub async fn add_block(
        &self,
        block: UnverifiedFullBlock,
        validation: BlockValidationContext,
    ) -> Result<BlockImportResult, BlockImportError> {
        let block = self.pre_validate(block, validation.clone()).await?;
        self.verify_apply(block, validation).await
    }

    pub async fn pre_validate(
        &self,
        block: UnverifiedFullBlock,
        validation: BlockValidationContext,
    ) -> Result<PreValidatedBlock, BlockImportError> {
        pre_validate(&self.pool, block, validation).await
    }

    pub async fn verify_apply(
        &self,
        block: PreValidatedBlock,
        validation: BlockValidationContext,
    ) -> Result<BlockImportResult, BlockImportError> {
        let result = self.verify_apply.verify_apply(block, validation).await?;
        // Flush step.
        let force = self.always_force_flush;
        self.backend
            .maybe_flush(force)
            .map_err(|err| BlockImportError::Internal(format!("DB flushing error: {err:#}").into()))?;
        self.metrics.update(&result.header, &self.backend);
        Ok(result)
    }

    pub async fn pre_validate_pending(
        &self,
        block: UnverifiedPendingFullBlock,
        validation: BlockValidationContext,
    ) -> Result<PreValidatedPendingBlock, BlockImportError> {
        pre_validate_pending(&self.pool, block, validation).await
    }

    pub async fn verify_apply_pending(
        &self,
        block: PreValidatedPendingBlock,
        validation: BlockValidationContext,
    ) -> Result<PendingBlockImportResult, BlockImportError> {
        self.verify_apply.verify_apply_pending(block, validation).await
    }
}
