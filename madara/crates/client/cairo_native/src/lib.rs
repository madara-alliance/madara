//! Cairo Native execution and compilation for Madara
//!
//! This crate provides Cairo Native execution as an alternative to Cairo VM for executing
//! Starknet contracts. It implements a multi-tier caching system with compilation management,
//! error handling, and comprehensive metrics tracking.
//!
//! **Important**: Native execution is only enabled when the `--enable-native-execution` CLI flag
//! is set to `true`. When disabled (default), all contracts use Cairo VM execution regardless
//! of cache state or compilation availability.
//!
//! # Module Organization
//!
//! - [`config`]: Configuration management for native execution settings
//! - [`error`]: Error types for compilation and execution failures
//! - [`metrics`]: Performance and operational metrics tracking
//! - [`cache`]: In-memory and disk-based caching of compiled classes
//! - [`compilation`]: Compilation orchestration (blocking/async modes, retry logic)
//! - [`execution`]: Main execution flow and class handling
//! - [`to_blockifier`]: Conversion to Blockifier's `RunnableCompiledClass` with native execution support

pub mod cache;
pub mod compilation;
pub mod config;
pub mod error;
pub mod execution;
pub mod metrics;
pub mod to_blockifier;

#[cfg(test)]
mod test_utils;

// Re-export commonly used types
pub use compilation::init_compilation_semaphore;
pub use config::{NativeCompilationMode, NativeConfig};
pub use error::NativeCompilationError;
