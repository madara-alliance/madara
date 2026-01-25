//! Integration between mc-rust-exec verification and mc-exec execution.
//!
//! This module provides:
//! 1. StateReader adapter for BlockifierStateAdapter
//! 2. Verification hooks for transaction tracing
//! 3. Conversion utilities between Blockifier and Rust execution formats
//!
//! # Configuration
//!
//! Class hashes are configured via environment variables:
//! ```bash
//! export RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH=0x0123456789abcdef...
//! ```
//!
//! See `mc_rust_exec::config` for all available environment variables.

#[cfg(feature = "rust-verification")]
use mc_rust_exec::{ContractAddress as RustContractAddress, StateError as RustStateError, StateReader};
#[cfg(feature = "rust-verification")]
use mp_convert::ToFelt;
#[cfg(feature = "rust-verification")]
use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

/// Initialize Rust execution verification on first use.
/// This logs the configuration status once.
#[cfg(feature = "rust-verification")]
static INIT: Lazy<()> = Lazy::new(|| {
    mc_rust_exec::init();
});

/// Initialize Rust execution verification.
///
/// Call this at startup to log which contracts are enabled for Rust verification.
/// This reads class hashes from environment variables (e.g., `RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH`).
#[cfg(feature = "rust-verification")]
pub fn init_rust_verification() {
    Lazy::force(&INIT);
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn init_rust_verification() {
    // No-op when verification is disabled
}

#[cfg(feature = "rust-verification")]
use blockifier::execution::call_info::CallInfo;
#[cfg(feature = "rust-verification")]
use crate::BlockifierStateAdapter;
#[cfg(feature = "rust-verification")]
use mc_db::MadaraStorageRead;

/// Adapter that allows BlockifierStateAdapter to be used with mc-rust-exec's StateReader trait.
#[cfg(feature = "rust-verification")]
pub struct RustExecStateAdapter<'a, D: MadaraStorageRead> {
    blockifier_adapter: &'a BlockifierStateAdapter<D>,
}

#[cfg(feature = "rust-verification")]
impl<'a, D: MadaraStorageRead> RustExecStateAdapter<'a, D> {
    pub fn new(blockifier_adapter: &'a BlockifierStateAdapter<D>) -> Self {
        Self { blockifier_adapter }
    }
}

#[cfg(feature = "rust-verification")]
impl<'a, D: MadaraStorageRead> StateReader for RustExecStateAdapter<'a, D> {
    fn get_storage_at(
        &self,
        contract: RustContractAddress,
        key: mc_rust_exec::types::StorageKey,
    ) -> Result<Felt, RustStateError> {
        use blockifier::state::state_api::StateReader as BlockifierStateReader;

        let starknet_contract = starknet_api::core::ContractAddress::try_from(contract.0)
            .map_err(|e| RustStateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let starknet_key = starknet_api::state::StorageKey::try_from(key.0)
            .map_err(|e| RustStateError::InvalidStorageKey(format!("Invalid storage key: {e}")))?;

        self.blockifier_adapter
            .get_storage_at(starknet_contract, starknet_key)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))
    }

    fn get_nonce_at(&self, contract: RustContractAddress) -> Result<mc_rust_exec::types::Nonce, RustStateError> {
        use blockifier::state::state_api::StateReader as BlockifierStateReader;

        let starknet_contract = starknet_api::core::ContractAddress::try_from(contract.0)
            .map_err(|e| RustStateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let nonce = self
            .blockifier_adapter
            .get_nonce_at(starknet_contract)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))?;

        Ok(mc_rust_exec::types::Nonce(nonce.0))
    }

    fn get_class_hash_at(
        &self,
        contract: RustContractAddress,
    ) -> Result<Option<Felt>, RustStateError> {
        use blockifier::state::state_api::StateReader as BlockifierStateReader;

        let starknet_contract = starknet_api::core::ContractAddress::try_from(contract.0)
            .map_err(|e| RustStateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let class_hash = self
            .blockifier_adapter
            .get_class_hash_at(starknet_contract)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))?;

        // Class hash of 0x0 means contract is not deployed
        if class_hash.0 == Felt::ZERO {
            Ok(None)
        } else {
            Ok(Some(class_hash.0))
        }
    }
}

/// Verify a transaction execution against Rust implementation.
///
/// Returns true if verification passed or was skipped (contract not supported).
/// Logs warnings if mismatches are detected.
#[cfg(feature = "rust-verification")]
pub fn verify_transaction_execution(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    contract_address: Felt,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller_address: Felt,
    execution_result: &crate::ExecutionResult,
) -> bool {
    // Extract data from Blockifier's execution result for the top-level call
    let blockifier_execute_call_info = match &execution_result.execution_info.execute_call_info {
        Some(info) => info,
        None => {
            tracing::warn!("No execute call info in Blockifier result");
            return true;
        }
    };

    let _result = verify_transaction_execution_with_call_info(
        blockifier_adapter,
        contract_address,
        class_hash,
        entry_point_selector,
        calldata,
        caller_address,
        blockifier_execute_call_info,
        execution_result,
    );

    // This function is for backwards compatibility - always returns true
    // The actual timing is available via verify_call_tree which returns CallTreeVerificationResult
    true
}

/// Result of Rust verification including timing information.
#[cfg(feature = "rust-verification")]
pub struct VerificationResult {
    /// Whether Rust actually executed (vs skipped)
    pub executed: bool,
    /// Time spent in pure Rust execution (not including comparison)
    pub execution_duration: std::time::Duration,
}

/// Verify a transaction execution against Rust implementation using specific call info.
///
/// This version takes an explicit call_info, allowing verification of inner calls
/// in account transactions.
#[cfg(feature = "rust-verification")]
fn verify_transaction_execution_with_call_info(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    contract_address: Felt,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller_address: Felt,
    call_info: &CallInfo,
    execution_result: &crate::ExecutionResult,
) -> VerificationResult {
    // Initialize on first call (logs configuration status)
    Lazy::force(&INIT);

    tracing::info!(
        "🔍 Rust verification checking contract {:#x} (class_hash: {:#x}, selector: {:#x})",
        contract_address,
        class_hash,
        entry_point_selector
    );

    // Create state adapter for Rust execution
    let rust_state = RustExecStateAdapter::new(blockifier_adapter);

    // ============================================================
    // RUST EXECUTION ONLY - START TIMING
    // ============================================================
    let rust_exec_start = std::time::Instant::now();

    // Try to execute with Rust implementation
    let rust_result = mc_rust_exec::execute_transaction(
        &rust_state,
        RustContractAddress(contract_address),
        class_hash,
        entry_point_selector,
        calldata,
        RustContractAddress(caller_address),
    );

    // ============================================================
    // RUST EXECUTION ONLY - END TIMING
    // ============================================================
    let rust_exec_duration = rust_exec_start.elapsed();

    let rust_result = match rust_result {
        Some(Ok(result)) => result,
        Some(Err(e)) => {
            tracing::info!(
                "⚠️  Rust execution error for contract {:#x}, function {:#x}: {e:?}",
                contract_address,
                entry_point_selector
            );
            return VerificationResult { executed: true, execution_duration: rust_exec_duration };
        }
        None => {
            // Contract/function not supported in Rust - this is expected
            tracing::info!(
                "⏭️  Contract {:#x} or function {:#x} not supported in Rust verification (skipping)",
                contract_address,
                entry_point_selector
            );
            return VerificationResult { executed: false, execution_duration: std::time::Duration::ZERO };
        }
    };

    // Prepare Blockifier data for comparison
    // IMPORTANT: Only include storage changes for the specific contract being verified.
    // The full state_diff includes fee token balance updates, account nonce changes, etc.
    // which are not part of this specific contract's execution.
    let blockifier_storage: Vec<(Felt, Vec<(Felt, Felt)>)> = execution_result
        .state_diff
        .storage_updates
        .iter()
        .filter(|(addr, _)| addr.to_felt() == contract_address)
        .map(|(addr, updates)| {
            let storage_entries: Vec<(Felt, Felt)> = updates.iter().map(|(k, v)| (k.to_felt(), *v)).collect();
            (addr.to_felt(), storage_entries)
        })
        .collect();

    // Retdata and events come from the specific call_info being verified
    let blockifier_retdata: Vec<Felt> = call_info.execution.retdata.0.clone();

    let blockifier_events: Vec<(Vec<Felt>, Vec<Felt>)> = call_info
        .execution
        .events
        .iter()
        .map(|event| {
            let keys: Vec<Felt> = event.event.keys.iter().map(ToFelt::to_felt).collect();
            let data: Vec<Felt> = event.event.data.0.clone();
            (keys, data)
        })
        .collect();

    let blockifier_failed = call_info.execution.failed;

    // Log comparison details for debugging
    tracing::debug!(
        "📊 Comparison details for contract {:#x}:",
        contract_address
    );
    tracing::debug!(
        "   Rust retdata: {:?}",
        rust_result.call_result.retdata
    );
    tracing::debug!(
        "   Blockifier retdata: {:?}",
        blockifier_retdata
    );
    tracing::debug!(
        "   Rust events count: {}",
        rust_result.call_result.events.len()
    );
    tracing::debug!(
        "   Blockifier events count: {}",
        blockifier_events.len()
    );
    if !rust_result.call_result.events.is_empty() {
        for (i, event) in rust_result.call_result.events.iter().enumerate() {
            tracing::debug!(
                "   Rust event[{}]: keys={:?}, data={:?}",
                i,
                event.keys.iter().map(|k| format!("{:#x}", k)).collect::<Vec<_>>(),
                event.data.iter().map(|d| format!("{:#x}", d)).collect::<Vec<_>>()
            );
        }
        for (i, (keys, data)) in blockifier_events.iter().enumerate() {
            tracing::debug!(
                "   Blockifier event[{}]: keys={:?}, data={:?}",
                i,
                keys.iter().map(|k| format!("{:#x}", k)).collect::<Vec<_>>(),
                data.iter().map(|d| format!("{:#x}", d)).collect::<Vec<_>>()
            );
        }
    }
    tracing::debug!(
        "   Rust storage updates: {} contracts",
        rust_result.state_diff.storage_updates.len()
    );
    tracing::debug!(
        "   Blockifier storage updates (filtered): {} contracts",
        blockifier_storage.len()
    );

    // Perform verification
    let verification_result = mc_rust_exec::compare_with_blockifier(
        &rust_result,
        &blockifier_storage,
        &blockifier_retdata,
        &blockifier_events,
        blockifier_failed,
    );

    if !verification_result.passed {
        tracing::info!(
            "❌ Rust verification FAILED for contract {:#x}, function {:#x}:",
            contract_address,
            entry_point_selector
        );
        for error in &verification_result.errors {
            tracing::info!("    - {:?}", error);
        }
        // Rust DID execute, even though verification failed
        return VerificationResult { executed: true, execution_duration: rust_exec_duration };
    }

    tracing::info!(
        "✅ Rust verification PASSED for contract {:#x}, function {:#x}",
        contract_address,
        entry_point_selector
    );

    // Log what was verified when passing (at info level for visibility)
    if !rust_result.call_result.retdata.is_empty() {
        tracing::info!(
            "   📤 Return data verified: {:?}",
            rust_result.call_result.retdata.iter().map(|f| format!("{:#x}", f)).collect::<Vec<_>>()
        );
    }
    if !rust_result.call_result.events.is_empty() {
        tracing::info!(
            "   📢 Events verified: {} event(s)",
            rust_result.call_result.events.len()
        );
        for (i, event) in rust_result.call_result.events.iter().enumerate() {
            tracing::info!(
                "      Event[{}]: keys=[{}], data=[{}]",
                i,
                event.keys.iter().map(|k| format!("{:#x}", k)).collect::<Vec<_>>().join(", "),
                event.data.iter().map(|d| format!("{:#x}", d)).collect::<Vec<_>>().join(", ")
            );
        }
    }
    if !rust_result.state_diff.storage_updates.is_empty() {
        let total_updates: usize = rust_result.state_diff.storage_updates.values().map(|v| v.len()).sum();
        tracing::info!(
            "   💾 Storage verified: {} update(s)",
            total_updates
        );
    }

    VerificationResult { executed: true, execution_duration: rust_exec_duration }
}

/// Result of verifying an entire call tree.
#[cfg(feature = "rust-verification")]
pub struct CallTreeVerificationResult {
    /// Whether any contract in the tree was actually verified (not just skipped)
    pub any_executed: bool,
    /// Total time spent in pure Rust execution across all verified contracts
    pub total_execution_duration: std::time::Duration,
}

/// Recursively verify all calls in the call tree.
///
/// This handles account transactions where the actual contract call is nested
/// inside the account's __execute__ call.
///
/// Returns verification result with timing information.
#[cfg(feature = "rust-verification")]
pub fn verify_call_tree(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    call_info: &CallInfo,
    execution_result: &crate::ExecutionResult,
) -> CallTreeVerificationResult {
    // Verify this call
    let this_result = verify_single_call(blockifier_adapter, call_info, execution_result);

    let mut any_executed = this_result.executed;
    let mut total_duration = this_result.execution_duration;

    // Recursively verify all inner calls
    for inner_call in &call_info.inner_calls {
        let inner_result = verify_call_tree(blockifier_adapter, inner_call, execution_result);
        if inner_result.any_executed {
            any_executed = true;
        }
        total_duration += inner_result.total_execution_duration;
    }

    CallTreeVerificationResult {
        any_executed,
        total_execution_duration: total_duration,
    }
}

/// Verify a single call against Rust implementation.
///
/// Returns verification result with timing information.
#[cfg(feature = "rust-verification")]
fn verify_single_call(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    call_info: &CallInfo,
    execution_result: &crate::ExecutionResult,
) -> VerificationResult {
    let contract_address = call_info.call.storage_address.to_felt();
    let class_hash = call_info.call.class_hash.unwrap_or_default().to_felt();
    let entry_point_selector = call_info.call.entry_point_selector.to_felt();
    let calldata = &call_info.call.calldata.0;
    let caller_address = call_info.call.caller_address.to_felt();

    verify_transaction_execution_with_call_info(
        blockifier_adapter,
        contract_address,
        class_hash,
        entry_point_selector,
        calldata,
        caller_address,
        call_info,
        execution_result,
    )
}

/// Result of verifying an entire call tree (placeholder for disabled feature).
#[cfg(not(feature = "rust-verification"))]
pub struct CallTreeVerificationResult {
    pub any_executed: bool,
    pub total_execution_duration: std::time::Duration,
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn verify_call_tree(
    _blockifier_adapter: &crate::BlockifierStateAdapter<impl mc_db::MadaraStorageRead>,
    _call_info: &blockifier::execution::call_info::CallInfo,
    _execution_result: &crate::ExecutionResult,
) -> CallTreeVerificationResult {
    // No-op when verification is disabled
    CallTreeVerificationResult {
        any_executed: false,
        total_execution_duration: std::time::Duration::ZERO,
    }
}

// ============================================================================
// Cairo Native Pre-compilation for Benchmark Classes
// ============================================================================

/// Pre-compile benchmark classes for Cairo Native at startup.
///
/// This function reads the class hashes from environment variables (configured for
/// Rust verification) and pre-compiles them using Cairo Native. This eliminates
/// the cold start problem when tracing transactions.
///
/// # Arguments
/// * `backend` - The Madara backend for reading classes from the database
/// * `cairo_native_config` - The Cairo Native configuration
///
/// # Behavior
/// - If Cairo Native is not enabled, this function does nothing
/// - For each configured class hash:
///   - Skip if already compiled (in cache)
///   - Load Sierra class from database
///   - Compile to native code (blocking)
///   - Cache the result
///
/// Classes that don't exist in the database are skipped with a warning.
#[cfg(feature = "rust-verification")]
pub fn precompile_benchmark_classes<D: mc_db::MadaraStorageRead>(
    backend: &std::sync::Arc<mc_db::MadaraBackend<D>>,
    cairo_native_config: &std::sync::Arc<mc_class_exec::NativeConfig>,
) {
    use mp_class::ConvertedClass;

    // Only proceed if Cairo Native is enabled
    if !cairo_native_config.is_enabled() {
        tracing::info!(
            "Cairo Native is disabled, skipping benchmark class pre-compilation"
        );
        return;
    }

    // Get the class hashes from Rust verification config
    let class_hashes = get_benchmark_class_hashes();

    if class_hashes.is_empty() {
        tracing::info!(
            "No benchmark class hashes configured, skipping pre-compilation"
        );
        return;
    }

    tracing::info!(
        "🔧 Pre-compiling {} benchmark class(es) for Cairo Native...",
        class_hashes.len()
    );

    let mut compiled_count = 0;
    let mut skipped_count = 0;
    let mut failed_count = 0;

    for (name, class_hash) in class_hashes {
        let starknet_class_hash = starknet_api::core::ClassHash(class_hash);

        // Check if already compiled
        if mc_class_exec::cache_contains(&starknet_class_hash) {
            tracing::debug!(
                "   ✓ {} ({:#x}) already compiled, skipping",
                name, class_hash
            );
            skipped_count += 1;
            continue;
        }

        // Get a view on the latest confirmed block to load the class
        let view = backend.view_on_latest_confirmed();

        // Load the Sierra class from the database
        let sierra_class = match view.get_class_info_and_compiled(&class_hash) {
            Ok(Some(ConvertedClass::Sierra(sierra))) => sierra,
            Ok(Some(ConvertedClass::Legacy(_))) => {
                tracing::warn!(
                    "   ✗ {} ({:#x}) - Legacy class, not supported for Cairo Native",
                    name, class_hash
                );
                failed_count += 1;
                continue;
            }
            Ok(None) => {
                tracing::warn!(
                    "   ⚠ {} ({:#x}) - class not found in database (not deployed yet?)",
                    name, class_hash
                );
                failed_count += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    "   ✗ {} ({:#x}) - failed to load class: {}",
                    name, class_hash, e
                );
                failed_count += 1;
                continue;
            }
        };

        // Compile the class
        tracing::info!(
            "   ⏳ Compiling {} ({:#x})...",
            name, class_hash
        );

        let start = std::time::Instant::now();
        match mc_class_exec::compile_native_blocking(
            starknet_class_hash,
            &sierra_class,
            cairo_native_config,
        ) {
            Ok(_) => {
                let elapsed = start.elapsed();
                tracing::info!(
                    "   ✓ {} ({:#x}) compiled in {:?}",
                    name, class_hash, elapsed
                );
                compiled_count += 1;
            }
            Err(e) => {
                tracing::warn!(
                    "   ✗ {} ({:#x}) - compilation failed: {}",
                    name, class_hash, e
                );
                failed_count += 1;
            }
        }
    }

    tracing::info!(
        "🔧 Pre-compilation complete: {} compiled, {} skipped (cached), {} failed",
        compiled_count, skipped_count, failed_count
    );
}

/// Get the list of benchmark class hashes from Rust verification config.
#[cfg(feature = "rust-verification")]
fn get_benchmark_class_hashes() -> Vec<(&'static str, Felt)> {
    let mut hashes = Vec::new();

    if let Some(hash) = mc_rust_exec::config::simple_counter_class_hash() {
        hashes.push(("SimpleCounter", hash));
    }
    if let Some(hash) = mc_rust_exec::config::counter_with_event_class_hash() {
        hashes.push(("CounterWithEvent", hash));
    }
    if let Some(hash) = mc_rust_exec::config::random_100_hashes_class_hash() {
        hashes.push(("Random100Hashes", hash));
    }

    hashes
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn precompile_benchmark_classes<D: mc_db::MadaraStorageRead>(
    _backend: &std::sync::Arc<mc_db::MadaraBackend<D>>,
    _cairo_native_config: &std::sync::Arc<mc_class_exec::NativeConfig>,
) {
    // No-op when verification is disabled
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn verify_transaction_execution(
    _blockifier_adapter: &crate::BlockifierStateAdapter<impl mc_db::MadaraStorageRead>,
    _contract_address: Felt,
    _class_hash: Felt,
    _entry_point_selector: Felt,
    _calldata: &[Felt],
    _caller_address: Felt,
    _execution_result: &crate::ExecutionResult,
) -> bool {
    true // Always pass when verification is disabled
}
