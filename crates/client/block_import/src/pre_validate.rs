use crate::{
    BlockImportError, PreValidatedBlock, PreValidatedPendingBlock, RayonPool, UnverifiedFullBlock,
    UnverifiedPendingFullBlock,
};

use dp_validation::{PreValidate, ValidationContext};

/// This function wraps the [`block.pre_validate`] step, which runs on the rayon pool, in a tokio-friendly future.
pub async fn pre_validate(
    pool: &RayonPool,
    block: UnverifiedFullBlock,
    validation_context: ValidationContext,
) -> Result<PreValidatedBlock, BlockImportError> {
    pool.spawn_rayon_task(move || block.pre_validate(&validation_context)).await
}

/// See [`pre_validate`].
pub async fn pre_validate_pending(
    pool: &RayonPool,
    block: UnverifiedPendingFullBlock,
    validation_context: ValidationContext,
) -> Result<PreValidatedPendingBlock, BlockImportError> {
    pool.spawn_rayon_task(move || block.pre_validate(&validation_context)).await
}
