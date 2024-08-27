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

use std::sync::Arc;

use dc_db::DeoxysBackend;
use dp_block::{PreValidatedBlock, PreValidatedPendingBlock, UnverifiedFullBlock, UnverifiedFullPendingBlock};
use dp_rayon_pool::RayonPool;
use dp_validation::{Validate, ValidationContext};

mod error;
mod verify_apply;

pub use error::*;
pub use rayon::*;
pub use verify_apply::*;

pub struct BlockImporter {
    pool: Arc<RayonPool>,
    verify_apply: VerifyApply,
}

impl BlockImporter {
    pub fn new(backend: Arc<DeoxysBackend>) -> Self {
        let pool = Arc::new(RayonPool::new());
        Self { verify_apply: VerifyApply::new(Arc::clone(&backend), Arc::clone(&pool)), pool }
    }

    pub async fn pre_validate(
        &self,
        block: UnverifiedFullBlock,
        validation_context: ValidationContext,
    ) -> Result<PreValidatedBlock, BlockImportError> {
        block.spawn_validate(&self.pool, validation_context).await.map_err(|_| BlockImportError::TODO)
    }

    pub async fn verify_apply(
        &self,
        block: PreValidatedBlock,
        validation_context: ValidationContext,
    ) -> Result<BlockImportResult, BlockImportError> {
        self.verify_apply.verify_apply(block, validation_context).await
    }

    pub async fn pre_validate_pending(
        &self,
        block: UnverifiedFullPendingBlock,
        validation_context: ValidationContext,
    ) -> Result<PreValidatedPendingBlock, BlockImportError> {
        block.spawn_validate(&self.pool, validation_context).await.map_err(|_| BlockImportError::TODO)
    }

    pub async fn verify_apply_pending(
        &self,
        block: PreValidatedPendingBlock,
        validation_context: ValidationContext,
    ) -> Result<PendingBlockImportResult, BlockImportError> {
        self.verify_apply.verify_apply_pending(block, validation_context).await
    }
}
