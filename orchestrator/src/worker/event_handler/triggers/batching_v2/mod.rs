pub mod aggregator;
pub mod snos;
pub mod utils;

// TODO: Update this to not have empty states in Accumulated and BatchCompleted's complete_state field
pub enum BlockProcessingResult<S> {
    /// Block added to current batch, continue accumulating
    Accumulated(S),

    /// Batch is complete, here's the finalized batch and fresh state for next batch
    BatchCompleted { completed_state: S, new_state: S },

    /// This can happen when the block is in pre-confirmed state
    /// Returns the current state unchanged so it can be saved
    NotBatched(S),
}
