use crate::{
    BlockImportError, PreValidatedBlock, PreValidatedPendingBlock, RayonPool, UnverifiedFullBlock,
    UnverifiedPendingFullBlock,
};

use dp_validation::{PreValidate, Validation};

/// This function wraps the [`block.pre_validate`] step, which runs on the rayon pool, in a tokio-friendly future.
pub async fn pre_validate(
    pool: &RayonPool,
    block: UnverifiedFullBlock,
    validation: Validation,
) -> Result<PreValidatedBlock, BlockImportError> {
    pool.spawn_rayon_task(move || block.pre_validate(&validation)).await
}

/// See [`pre_validate`].
pub async fn pre_validate_pending(
    pool: &RayonPool,
    block: UnverifiedPendingFullBlock,
    validation: Validation,
) -> Result<PreValidatedPendingBlock, BlockImportError> {
    pool.spawn_rayon_task(move || block.pre_validate(&validation)).await
}
