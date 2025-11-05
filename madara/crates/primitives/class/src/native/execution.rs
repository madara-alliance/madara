//! Execution flow for Cairo Native classes
//!
//! This module handles the main execution flow for Sierra classes with native execution support.

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use std::sync::Arc;
use std::time::Instant;

use super::config;
use crate::SierraConvertedClass;

use super::cache;
use super::compilation;

/// Handle Sierra class conversion with native execution support.
///
/// Implements the full caching and compilation flow for Sierra classes.
pub(crate) fn handle_sierra_class(
    sierra: &SierraConvertedClass,
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<crate::CompiledSierra>,
    info: &crate::SierraClassInfo,
) -> Result<RunnableCompiledClass, ProgramError> {
    let config = config::get_config();
    tracing::debug!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        native_enabled = config.enable_native_execution,
        "handle_sierra_class"
    );

    // If native execution is disabled, use VM immediately
    if !config.enable_native_execution {
        return handle_native_disabled(class_hash, compiled, info);
    }

    // Native execution enabled - try cache first, then compile if needed
    let start = Instant::now();

    // Check cache (memory then disk)
    if let Some(cached) = try_cache_lookup(class_hash, sierra, start) {
        return Ok(cached);
    }

    // Cache miss - record metric and proceed to compilation
    super::metrics::metrics().record_cache_miss();

    // Handle compilation based on mode (blocking vs async)
    let config = config::get_config();
    if config.is_blocking_mode() {
        handle_blocking_compilation(class_hash, sierra)
    } else {
        handle_async_compilation(class_hash, sierra, compiled, info)
    }
}

/// Handles the case when native execution is disabled.
///
/// Converts the Sierra class to VM format and returns it immediately.
fn handle_native_disabled(
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<crate::CompiledSierra>,
    info: &crate::SierraClassInfo,
) -> Result<RunnableCompiledClass, ProgramError> {
    tracing::debug!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "disabled_fallback_to_vm"
    );
    convert_to_vm_class(class_hash, compiled, info, "sierra_version_error", "vm_conversion_error")
}

/// Attempts to find the class in cache (memory first, then disk).
///
/// Returns `Some(cached_class)` if found, `None` if not cached.
fn try_cache_lookup(
    class_hash: &starknet_types_core::felt::Felt,
    sierra: &SierraConvertedClass,
    start: Instant,
) -> Option<RunnableCompiledClass> {
    // Check in-memory cache first (fastest)
    if let Some(cached) = cache::try_get_from_memory_cache(class_hash) {
        let elapsed = start.elapsed();
        tracing::info!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            elapsed = ?elapsed,
            "native_from_memory"
        );
        return Some(cached);
    }

    // Try disk cache (slower but persistent)
    match cache::try_get_from_disk_cache(class_hash, sierra) {
        Ok(Some(cached)) => {
            let elapsed = start.elapsed();
            tracing::info!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                elapsed = ?elapsed,
                "native_from_disk"
            );
            Some(cached)
        }
        Ok(None) => {
            // Cache miss - will fall through to compilation
            None
        }
        Err(e) => {
            // Error loading from disk - log and fall through to compilation
            let elapsed = start.elapsed();
            tracing::warn!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %e,
                elapsed = ?elapsed,
                "disk_cache_error_fallback"
            );
            None
        }
    }
}

/// Handles compilation in blocking mode.
///
/// Compiles synchronously and returns the native class, or fails if compilation fails.
fn handle_blocking_compilation(
    class_hash: &starknet_types_core::felt::Felt,
    sierra: &SierraConvertedClass,
) -> Result<RunnableCompiledClass, ProgramError> {
    tracing::info!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "compilation_blocking_miss"
    );

    match compilation::compile_native_blocking(*class_hash, sierra, None) {
        Ok(native_class) => {
            tracing::info!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                "compilation_blocking_complete"
            );
            let runnable = RunnableCompiledClass::from(native_class.as_ref().clone());
            tracing::debug!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                "returning_native_blocking"
            );
            Ok(runnable)
        }
        Err(e) => {
            // In blocking mode, failures are fatal
            tracing::error!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %e,
                "compilation_blocking_failed"
            );
            Err(e.into())
        }
    }
}

/// Handles compilation in async mode.
///
/// Spawns background compilation and immediately falls back to VM execution.
fn handle_async_compilation(
    class_hash: &starknet_types_core::felt::Felt,
    sierra: &SierraConvertedClass,
    compiled: &Arc<crate::CompiledSierra>,
    info: &crate::SierraClassInfo,
) -> Result<RunnableCompiledClass, ProgramError> {
    // Check if this is a retry of a previously failed compilation
    let should_retry = compilation::FAILED_COMPILATIONS.remove(class_hash).is_some();
    if should_retry {
        tracing::info!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            "compilation_retrying_previously_failed"
        );
    }

    // Spawn compilation task if not already in progress
    spawn_compilation_if_needed(class_hash, sierra);

    // Fall back to VM while compilation happens in background
    super::metrics::metrics().record_vm_fallback();
    tracing::info!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "fallback_to_vm_async"
    );
    convert_to_vm_class(class_hash, compiled, info, "async_sierra_version_error", "async_vm_conversion_error")
}

/// Spawns a background compilation task if one isn't already running for this class.
///
/// Uses atomic operations to prevent race conditions when multiple requests arrive simultaneously.
fn spawn_compilation_if_needed(class_hash: &starknet_types_core::felt::Felt, sierra: &SierraConvertedClass) {
    // Check if already compiling - use entry API for atomic check-and-insert
    if compilation::COMPILATION_IN_PROGRESS.contains_key(class_hash) {
        tracing::debug!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            "compilation_already_in_progress"
        );
        return;
    }

    use dashmap::mapref::entry::Entry;
    match compilation::COMPILATION_IN_PROGRESS.entry(*class_hash) {
        Entry::Vacant(entry) => {
            // Not compiling - mark as in progress and spawn task
            use std::sync::Arc;
            use tokio::sync::RwLock;
            entry.insert(Arc::new(RwLock::new(())));

            let in_progress_count = compilation::COMPILATION_IN_PROGRESS.len();
            let max_concurrent = config::get_max_concurrent_compilations();

            tracing::debug!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                in_progress = in_progress_count,
                max_concurrent = max_concurrent,
                "compilation_async_spawning"
            );
            compilation::spawn_native_compilation(*class_hash, Arc::new(sierra.clone()));
        }
        Entry::Occupied(_) => {
            // Race condition: another thread started compilation
            tracing::debug!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                "compilation_already_in_progress"
            );
        }
    }
}

/// Converts a CompiledSierra to RunnableCompiledClass for VM execution.
///
/// This is used when native execution is disabled or as a fallback during async compilation.
fn convert_to_vm_class(
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<crate::CompiledSierra>,
    info: &crate::SierraClassInfo,
    sierra_version_error_label: &str,
    vm_conversion_error_label: &str,
) -> Result<RunnableCompiledClass, ProgramError> {
    let sierra_version = info.contract_class.sierra_version().map_err(|e| {
        tracing::error!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            error = %e,
            "{}", sierra_version_error_label
        );
        ProgramError::Parse(serde::de::Error::custom("Failed to get sierra version from program"))
    })?;

    let vm_class = RunnableCompiledClass::try_from(ApiContractClass::V1((
        compiled.as_ref().try_into().map_err(|e| {
            tracing::error!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %format!("{:?}", e),
                "{}", vm_conversion_error_label
            );
            ProgramError::Parse(serde::de::Error::custom(format!("Failed to convert to VM class: {:?}", e)))
        })?,
        sierra_version,
    )))?;

    tracing::debug!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "returning_vm_class"
    );
    Ok(vm_class)
}
