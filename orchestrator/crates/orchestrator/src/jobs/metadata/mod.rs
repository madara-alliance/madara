//! Job metadata types and their management.
//!
//! This module defines the metadata structures used by different job types in the system.
//! Each job type has its specific metadata requirements, and the fields are managed either
//! by workers during job creation or by jobs during processing.

mod common;
mod da;
mod proving;
mod snos;
mod state_update;

// Re-export everything for backward compatibility
use color_eyre::eyre;
use color_eyre::eyre::eyre;
pub use common::*;
pub use da::*;
pub use proving::*;
use serde::{Deserialize, Serialize};
pub use snos::*;
pub use state_update::*;

/// Enum containing all possible job-specific metadata types.
///
/// This enum is used to provide type-safe access to job-specific metadata
/// while maintaining a common interface for job processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum JobSpecificMetadata {
    /// SNOS job metadata
    Snos(SnosMetadata),
    /// State update job metadata
    StateUpdate(StateUpdateMetadata),
    /// Proving job metadata
    Proving(ProvingMetadata),
    /// Data availability job metadata
    Da(DaMetadata),
}

/// Macro to implement TryInto for JobSpecificMetadata variants
macro_rules! impl_try_into_metadata {
    ($variant:ident, $type:ident) => {
        impl TryInto<$type> for JobSpecificMetadata {
            type Error = eyre::Error;

            fn try_into(self) -> Result<$type, Self::Error> {
                match self {
                    JobSpecificMetadata::$variant(metadata) => Ok(metadata),
                    _ => Err(eyre!(concat!("Invalid metadata type: expected ", stringify!($variant), " metadata"))),
                }
            }
        }
    };
}

// Implement TryInto for all metadata types
impl_try_into_metadata!(Snos, SnosMetadata);
impl_try_into_metadata!(Proving, ProvingMetadata);
impl_try_into_metadata!(Da, DaMetadata);
impl_try_into_metadata!(StateUpdate, StateUpdateMetadata);

/// Complete job metadata containing both common and job-specific fields.
///
/// # Field Management
/// - `common`: Managed automatically by the job processing system
/// - `specific`: Contains job-type specific fields managed by workers and jobs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobMetadata {
    /// Common metadata fields shared across all job types
    pub common: CommonMetadata,
    /// Job-specific metadata fields
    pub specific: JobSpecificMetadata,
}
