/// Greedy mode worker tests
///
/// This module contains comprehensive tests for the greedy (queue-less) worker mode.
/// Greedy mode is now the default and only mode - jobs are polled directly from MongoDB
/// with atomic claiming instead of being consumed from SQS queues.
///
/// Test coverage includes:
/// - Atomic job claiming with single and multiple orchestrators
/// - Job priority (PendingRetry before Created, oldest first)
/// - Concurrency and race condition handling
/// - Orphan job detection and recovery
/// - Job creation caps to prevent MongoDB overflow
/// - Multi-orchestrator scenarios
pub mod claiming;
pub mod concurrency;
pub mod job_creation_caps;
pub mod orphan_recovery;
