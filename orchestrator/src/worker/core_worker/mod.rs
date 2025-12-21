/// Worker implementation for queue-less job processing
///
/// This module implements the worker pattern where workers actively poll
/// MongoDB for available jobs instead of consuming from SQS. Jobs are claimed
/// atomically using MongoDB's findOneAndUpdate operation.
///
/// Key features:
/// - Atomic job claiming to prevent race conditions
/// - Configurable polling intervals
/// - Per-job-type concurrency limits
/// - Graceful shutdown with in-flight job tracking
/// - Exponential backoff on database errors
/// - Backward compatibility with SQS-based jobs
/// - Split processing and verification workers running in parallel
pub mod config;
pub mod controller;
pub mod metrics;
pub mod processing_worker;
pub mod verification_worker;

pub use config::WorkerConfig;
pub use controller::WorkerController;
pub use processing_worker::ProcessingWorker;
pub use verification_worker::VerificationWorker;
