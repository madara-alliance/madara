/// Worker mode worker tests
///
/// Tests for the worker architecture where orchestrators
/// poll MongoDB directly using atomic operations instead of consuming from SQS.

#[cfg(test)]
pub mod basic;

#[cfg(test)]
pub mod claiming;

#[cfg(test)]
pub mod concurrency;

#[cfg(test)]
pub mod orphan_recovery;

#[cfg(test)]
pub mod job_creation_caps;

#[cfg(test)]
pub mod error_handling;

// #[cfg(test)]
// pub mod retry_limits;  // TODO: Requires mock setup for job handlers
