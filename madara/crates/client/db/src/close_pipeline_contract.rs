/// Metadata returned when a close job is queued by the close pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedMeta {
    pub block_n: u64,
    pub queue_depth: usize,
}

/// Result contract for close_preconfirmed in the queue-first architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePreconfirmedResult {
    Queued(QueuedMeta),
}

/// Payload for queued close jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseJobPayload {
    pub block_n: u64,
}
