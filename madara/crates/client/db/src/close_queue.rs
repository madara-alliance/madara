/// Metadata returned when a close job is accepted by the close queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedMeta {
    pub block_n: u64,
    pub queue_depth: usize,
}

/// Result contract for close_preconfirmed in the queued architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePreconfirmedResult {
    Queued(QueuedMeta),
}

/// Skeleton payload for queued close jobs.
///
/// Phase 0 intentionally keeps this minimal; additional fields are introduced in Phase 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseJobPayload {
    pub block_n: u64,
}
