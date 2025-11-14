//! Madara sync. This crate manages state synchronization for Madara nodes. This includes fetching
//! blocks, transactions, and state updates from external sources and applying them to the local
//! database.
//!
//! # Overview
//!
//! The Sync module is responsible for downloading, verifying, and applying blocks
//! using a multi-stage pipeline architecture designed to maximize throughput. The module supports
//! multiple sync sources with some additional metrics.
//!
//! The sync system is split into two main layers: the outer [`SyncController`] handles
//! orchestration and configuration, while the inner [`PipelineController`]  implementations manages
//! the actual data processing workflows.
//!
//! # Sync Architecture
//!
//! The sync module uses a pipeline architecture with four main stages:
//!
//! - **Gateway Pipeline**: Downloads blocks and state updates from gateway endpoints.
//! - **Classes Pipeline**: Fetches and compiles smart contract classes.
//! - **Verification Pipeline**: Validates block integrity, commitments, and hashes.
//! - **State Application Pipeline**: Applies state diffs to the global Merkle trie.
//!
//! # Pipeline Processing Model
//!
//! Each pipeline implements the [`PipelineSteps`] trait with two execution phases:
//!
//! - **Parallel Phase**: CPU/IO intensive operations that can run concurrently across
//!   multiple blocks (downloading, verification, compilation).
//! - **Sequential Phase**: Operations that must be applied in strict block order
//!   (database updates, state root computation, head status changes).
//!
//! This design maximizes throughput by parallelizing expensive operations while ensuring
//! blockchain state consistency through an ordered application of changes.
//!
//! # Gateway Synchronization Flow
//!
//! When synchronizing from a Starknet gateway, the sync pipeline uses the following flow:
//!
//! 1. **Block Discovery**: The [`probe`] module continuously polls the gateway for new blocks to
//!    try and get an idea for where the tip of the chain is.
//! 2. **Blocks Resolution**: Multiple blocks are fetched concurrently via [`gateway::blocks`].
//! 3. **Class Resolution**: Smart contract classes are fetched concurrently via [`gateway::classes`].
//! 4. **Verification**: The [`import`] module validates all block data and commitments.
//! 5. **Sequential Storage**: Verified data is stored to the database in block order.
//! 6. **State Application**: State diffs are applied to the global trie via [`apply_state`].
//! 7. **Head Update**: The node's head status is updated to reflect the new state.
//!
//! # Block Import and Verification
//!
//! The [`BlockImporter`] is responsible for handling block validation:
//!
//! - **Header Verification**: Block hash, parent hash, and protocol version checks.
//! - **Transaction Integrity**: Transaction hash and commitment validation.
//! - **Receipt Verification**: Execution receipt and commitment validation.
//! - **Event Verification**: Event hash and commitment validation.
//! - **State Diff Validation**: State update commitment and length verification.
//! - **Class Compilation**: Sierra-to-CASM compilation and hash verification.
//!
//! The importer supports different validation modes for testing and production use:
//!
//! ```no_run
//! use mc_sync::import::{BlockImporter, BlockValidationConfig};
//!
//! // Production mode with full verification
//! let strict_config = BlockValidationConfig::default();
//!
//! // Testing mode with relaxed validation
//! let test_config = BlockValidationConfig::default()
//!     .all_verifications_disabled(true)
//!     .trust_class_hashes(true);
//!
//! let importer = BlockImporter::new(backend, strict_config);
//! ```
//!
//! # Sync Controller Configuration
//!
//! The [`SyncController`] supports different sync modes depending on the usecase:
//!
//! ```no_run
//! use mc_sync::{SyncControllerConfig, gateway::ForwardSyncConfig};
//!
//! // Full node sync with L1 verification
//! let config = SyncControllerConfig::default()
//!     .l1_head_recv(l1_receiver)
//!     .stop_on_sync(false);
//!
//! // Batch sync to specific height
//! let batch_config = SyncControllerConfig::default()
//!     .stop_at_block_n(Some(100000))
//!     .stop_on_sync(true)
//!     .no_pending_block(true);
//! ```
//!
//! # Performance and Monitoring
//!
//! The sync module provides monitoring through:
//!
//! ## Real-time Metrics via [`metrics`]
//!
//! - Block throughput and sync speed tracking.
//! - Database size and state trie metrics.
//! - Transaction and event count monitoring.
//! - L1 gas price tracking for cost analysis.
//!
//! ## Status Reporting via [`ServiceEvent`]
//!
//! - `Starting`: Initial sync startup.
//! - `SyncingTo { target }`: Active sync to target block.
//! - `Idle`: Waiting for new blocks.
//! - `UpdatedPendingBlock`: Pending block state changed.
//!
//! ## Rolling Average
//!
//! The [`counter`] module implements rolling average throughput calculation for performance
//! monitoring.
//!
//! # Testing Infrastructure
//!
//! The [`tests`] module provides several testing utilities:
//!
//! - **Gateway Mocking**: HTTP mock server for integration testing.
//! - **Pipeline Testing**: Isolated pipeline component verification.
//! - **E2E tests**: End-to-end sync testing with real block data.
//!
//! [`SyncController`]: crate::sync::SyncController
//! [`PipelineController`]: crate::pipeline::PipelineController
//! [`PipelineSteps`]: crate::pipeline::PipelineSteps
//! [`BlockImporter`]: crate::import::BlockImporter
//! [`probe`]: crate::probe
//! [`gateway::blocks`]: crate::gateway::blocks
//! [`gateway::classes`]: crate::gateway::classes
//! [`import`]: crate::import
//! [`apply_state`]: crate::apply_state
//! [`metrics`]: crate::metrics
//! [`ServiceEvent`]: crate::sync::ServiceEvent
//! [`counter`]: crate::counter
//! [`tests`]: crate::tests
mod apply_state;
mod counter;
mod metrics;
mod pipeline;
mod probe;
mod sync;
mod tests;
mod util;

pub use sync::SyncControllerConfig;

pub mod gateway;
pub mod import;
mod sync_utils;
