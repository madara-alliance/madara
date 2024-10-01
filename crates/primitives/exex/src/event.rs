use starknet_api::block::BlockNumber;

/// Events emitted by an `ExEx`.
pub enum ExExEvent {
    FinishedHeight(BlockNumber),
}
