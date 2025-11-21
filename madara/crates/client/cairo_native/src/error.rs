//! Error types for Cairo Native compilation
//!
//! This module defines the error types used throughout the Cairo Native compilation
//! pipeline. All errors can be converted to `ProgramError` for use in blockifier
//! execution contexts.
//!
//! # Error Categories
//!
//! - **Compilation errors**: Native compilation failures, timeouts
//! - **Conversion errors**: Sierra → CASM → Blockifier conversion failures
//! - **I/O errors**: Cache directory creation, disk load failures
//! - **Runtime errors**: Missing Tokio runtime context

use cairo_vm::types::errors::program_errors::ProgramError;
use serde::de::Error as _;
use std::time::Duration;

/// Errors that can occur during Cairo Native compilation.
///
/// All variants provide detailed error messages for debugging and observability.
/// Errors are automatically converted to `ProgramError` when propagated to blockifier.
#[derive(Debug, thiserror::Error)]
pub enum NativeCompilationError {
    #[error("Compilation failed: {0}")]
    CompilationFailed(String),

    #[error("Compilation timeout after {0:?}")]
    CompilationTimeout(Duration),

    #[error("Not in Tokio runtime context")]
    NoTokioRuntime,

    #[error("Failed to get sierra version: {0}")]
    SierraVersionError(String),

    #[error("Failed to convert to CASM: {0}")]
    CasmConversionError(String),

    #[error("Failed to convert to blockifier class: {0:?}")]
    BlockifierConversionError(String),

    #[error("Failed to create cache directory: {0}")]
    CacheDirectoryError(String),

    #[error("Failed to load native class from disk: {0}")]
    DiskLoadError(String),
}

impl From<NativeCompilationError> for ProgramError {
    fn from(e: NativeCompilationError) -> Self {
        ProgramError::Parse(serde_json::Error::custom(e.to_string()))
    }
}
