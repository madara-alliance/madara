pub mod aggregator;
pub mod snos;
pub mod utils;

pub enum BlockProcessingResult<S, NES> {
    /// Block added to current batch, continue accumulating
    Accumulated(NES),

    /// Batch is complete, here's the finalized batch and fresh state for next batch
    BatchCompleted { completed_state: NES, new_state: S },

    /// This can happen when the block is in pre-confirmed state
    /// Returns the current state unchanged so it can be saved if needed
    NotBatched(S),
}
