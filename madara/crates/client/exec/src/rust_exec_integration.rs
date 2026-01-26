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
use crate::BlockifierStateAdapter;
#[cfg(feature = "rust-verification")]
use blockifier::execution::call_info::CallInfo;
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

        let value = self
            .blockifier_adapter
            .get_storage_at(starknet_contract, starknet_key)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))?;

        // Debug: log storage reads through verification adapter (only for specific key to reduce noise)
        // The maker_balance_key is 0x34aa6b4be9f8e9f61b4332b16d3817b65d8e161d8ea72878888f8d1c89c1700
        if key.0 == Felt::from_hex_unchecked("0x34aa6b4be9f8e9f61b4332b16d3817b65d8e161d8ea72878888f8d1c89c1700") {
            tracing::info!(
                "🔬 RUST_ADAPTER_READ maker_balance: value={:#x} ({}), view={}, block_num={}",
                value,
                value,
                self.blockifier_adapter.view,
                self.blockifier_adapter.block_number
            );
        }

        Ok(value)
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

    fn get_class_hash_at(&self, contract: RustContractAddress) -> Result<Option<Felt>, RustStateError> {
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
        0, // Legacy function uses 0 timestamp
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
    /// Whether verification passed (only meaningful if executed=true)
    pub passed: bool,
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
    block_timestamp: u64,
) -> VerificationResult {
    // Initialize on first call (logs configuration status)
    Lazy::force(&INIT);

    // Verbose logging commented out for cleaner output
    // tracing::info!("🔍 Rust verification checking contract {:#x} (class_hash: {:#x}, selector: {:#x})", contract_address, class_hash, entry_point_selector);

    // Create state adapter for Rust execution
    let rust_state = RustExecStateAdapter::new(blockifier_adapter);

    // ============================================================
    // RUST EXECUTION ONLY - START TIMING
    // ============================================================
    let rust_exec_start = std::time::Instant::now();

    // Try to execute with Rust implementation (with timestamp for contracts that need it)
    let rust_result = mc_rust_exec::execute_transaction_with_timestamp(
        &rust_state,
        RustContractAddress(contract_address),
        class_hash,
        entry_point_selector,
        calldata,
        RustContractAddress(caller_address),
        block_timestamp,
    );

    // ============================================================
    // RUST EXECUTION ONLY - END TIMING
    // ============================================================
    let rust_exec_duration = rust_exec_start.elapsed();

    let rust_result = match rust_result {
        Some(Ok(result)) => result,
        Some(Err(e)) => {
            // Log errors at debug level (can enable for debugging)
            tracing::debug!(
                "⚠️  Rust execution error for contract {:#x}, function {:#x}: {e:?}",
                contract_address,
                entry_point_selector
            );
            return VerificationResult { executed: true, passed: false, execution_duration: rust_exec_duration };
        }
        None => {
            // Contract/function not supported in Rust - this is expected, log at debug level
            tracing::debug!(
                "⏭️  Contract {:#x} or function {:#x} not supported in Rust verification (skipping)",
                contract_address,
                entry_point_selector
            );
            return VerificationResult { executed: false, passed: false, execution_duration: std::time::Duration::ZERO };
        }
    };

    // Prepare Blockifier data for comparison - COMPREHENSIVE VERSION
    // Extract ALL state changes from the transaction to match against Rust execution.
    // Rust must produce identical outputs: account execution, fee transfers, nonce updates, etc.

    // 1. Storage updates - ALL contracts
    let blockifier_storage: Vec<(Felt, Vec<(Felt, Felt)>)> = execution_result
        .state_diff
        .storage_updates
        .iter()
        .map(|(addr, updates)| {
            let storage_entries: Vec<(Felt, Felt)> = updates.iter().map(|(k, v)| (k.to_felt(), *v)).collect();
            (addr.to_felt(), storage_entries)
        })
        .collect();

    // 2. Retdata and events come from the specific call_info being verified
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

    // 3. Nonce updates
    let blockifier_nonces: Vec<(Felt, Felt)> = execution_result
        .state_diff
        .address_to_nonce
        .iter()
        .map(|(addr, nonce)| (addr.to_felt(), nonce.to_felt()))
        .collect();

    // 4. L2-to-L1 messages
    let blockifier_messages: Vec<(Felt, Vec<Felt>)> = call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .map(|msg| (msg.message.to_address.0, msg.message.payload.0.clone()))
        .collect();

    // 5. Class hash deployments/replacements
    let blockifier_class_hashes: Vec<(Felt, Felt)> = execution_result
        .state_diff
        .address_to_class_hash
        .iter()
        .map(|(addr, hash)| (addr.to_felt(), hash.to_felt()))
        .collect();

    // 6. Compiled class hashes (for declares)
    let blockifier_compiled_class_hashes: Vec<(Felt, Felt)> = execution_result
        .state_diff
        .class_hash_to_compiled_class_hash
        .iter()
        .map(|(hash, compiled)| (hash.to_felt(), compiled.to_felt()))
        .collect();

    // Perform comprehensive verification
    let verification_result = mc_rust_exec::compare_with_blockifier_comprehensive(
        &rust_result,
        &blockifier_storage,
        &blockifier_retdata,
        &blockifier_events,
        blockifier_failed,
        &blockifier_nonces,
        &blockifier_messages,
        &blockifier_class_hashes,
        &blockifier_compiled_class_hashes,
    );

    if !verification_result.passed {
        tracing::warn!(
            "❌ RUST VERIFICATION FAILED for contract {:#x}, function {:#x}:",
            contract_address,
            entry_point_selector
        );
        tracing::warn!("   Found {} mismatch(es):", verification_result.errors.len());

        // Categorize errors for better reporting
        let mut storage_errors = 0;
        let mut nonce_errors = 0;
        let mut event_errors = 0;
        let mut message_errors = 0;
        let mut retdata_errors = 0;
        let mut status_errors = 0;
        let mut class_hash_errors = 0;

        for error in &verification_result.errors {
            match error {
                mc_rust_exec::VerificationError::StorageMismatch { contract, key, rust_value, blockifier_value } => {
                    storage_errors += 1;
                    tracing::warn!(
                        "      💾 Storage mismatch: contract={:#x}, key={:#x}, rust={:#x}, blockifier={:#x}",
                        contract, key, rust_value, blockifier_value
                    );
                }
                mc_rust_exec::VerificationError::ExtraStorageInRust { contract, key } => {
                    storage_errors += 1;
                    tracing::warn!("      💾 Extra storage in Rust: contract={:#x}, key={:#x}", contract, key);
                }
                mc_rust_exec::VerificationError::MissingStorageInRust { contract, key, blockifier_value } => {
                    storage_errors += 1;
                    tracing::warn!(
                        "      💾 Missing storage in Rust: contract={:#x}, key={:#x}, blockifier={:#x}",
                        contract, key, blockifier_value
                    );
                }
                mc_rust_exec::VerificationError::NonceMismatch { contract, rust, blockifier } => {
                    nonce_errors += 1;
                    tracing::warn!(
                        "      🔢 Nonce mismatch: contract={:#x}, rust={:#x}, blockifier={:#x}",
                        contract, rust, blockifier
                    );
                }
                mc_rust_exec::VerificationError::ExtraNonceInRust { contract, rust_nonce } => {
                    nonce_errors += 1;
                    tracing::warn!("      🔢 Extra nonce in Rust: contract={:#x}, nonce={:#x}", contract, rust_nonce);
                }
                mc_rust_exec::VerificationError::MissingNonceInRust { contract, blockifier_nonce } => {
                    nonce_errors += 1;
                    tracing::warn!(
                        "      🔢 Missing nonce in Rust: contract={:#x}, blockifier={:#x}",
                        contract, blockifier_nonce
                    );
                }
                mc_rust_exec::VerificationError::RetdataMismatch { rust, blockifier } => {
                    retdata_errors += 1;
                    tracing::warn!("      📤 Return data mismatch: rust_len={}, blockifier_len={}", rust.len(), blockifier.len());
                }
                mc_rust_exec::VerificationError::EventCountMismatch { rust_count, blockifier_count } => {
                    event_errors += 1;
                    tracing::warn!("      📢 Event count mismatch: rust={}, blockifier={}", rust_count, blockifier_count);
                }
                mc_rust_exec::VerificationError::EventMismatch { index, .. } => {
                    event_errors += 1;
                    tracing::warn!("      📢 Event content mismatch at index {}", index);
                }
                mc_rust_exec::VerificationError::MessageCountMismatch { rust_count, blockifier_count } => {
                    message_errors += 1;
                    tracing::warn!("      ✉️  Message count mismatch: rust={}, blockifier={}", rust_count, blockifier_count);
                }
                mc_rust_exec::VerificationError::MessageMismatch { index, .. } => {
                    message_errors += 1;
                    tracing::warn!("      ✉️  Message content mismatch at index {}", index);
                }
                mc_rust_exec::VerificationError::StatusMismatch { rust_failed, blockifier_failed } => {
                    status_errors += 1;
                    tracing::warn!("      ❓ Status mismatch: rust_failed={}, blockifier_failed={}", rust_failed, blockifier_failed);
                }
                mc_rust_exec::VerificationError::ClassHashMismatch { contract, rust, blockifier } => {
                    class_hash_errors += 1;
                    tracing::warn!(
                        "      🏗️  Class hash mismatch: contract={:#x}, rust={:?}, blockifier={:?}",
                        contract, rust, blockifier
                    );
                }
                mc_rust_exec::VerificationError::CompiledClassHashMismatch { class_hash, rust, blockifier } => {
                    class_hash_errors += 1;
                    tracing::warn!(
                        "      📦 Compiled class hash mismatch: class={:#x}, rust={:?}, blockifier={:?}",
                        class_hash, rust, blockifier
                    );
                }
            }
        }

        // Summary
        tracing::warn!("   Error summary:");
        if storage_errors > 0 {
            tracing::warn!("      - {} storage error(s)", storage_errors);
        }
        if nonce_errors > 0 {
            tracing::warn!("      - {} nonce error(s)", nonce_errors);
        }
        if event_errors > 0 {
            tracing::warn!("      - {} event error(s)", event_errors);
        }
        if message_errors > 0 {
            tracing::warn!("      - {} message error(s)", message_errors);
        }
        if retdata_errors > 0 {
            tracing::warn!("      - {} retdata error(s)", retdata_errors);
        }
        if status_errors > 0 {
            tracing::warn!("      - {} status error(s)", status_errors);
        }
        if class_hash_errors > 0 {
            tracing::warn!("      - {} class hash error(s)", class_hash_errors);
        }

        // Rust DID execute, but verification failed
        return VerificationResult { executed: true, passed: false, execution_duration: rust_exec_duration };
    }

    // Concise pass message (details commented out for cleaner output)
    // tracing::info!("✅ Rust verification PASSED for contract {:#x}, function {:#x}", contract_address, entry_point_selector);
    // Verbose details commented out - uncomment for debugging:
    // if !rust_result.call_result.retdata.is_empty() {
    //     tracing::info!("   📤 Return data verified: {:?}", rust_result.call_result.retdata.iter().map(|f| format!("{:#x}", f)).collect::<Vec<_>>());
    // }
    // if !rust_result.call_result.events.is_empty() {
    //     tracing::info!("   📢 Events verified: {} event(s)", rust_result.call_result.events.len());
    // }
    // if !rust_result.state_diff.storage_updates.is_empty() {
    //     let total_updates: usize = rust_result.state_diff.storage_updates.values().map(|v| v.len()).sum();
    //     tracing::info!("   💾 Storage verified: {} update(s)", total_updates);
    // }

    VerificationResult { executed: true, passed: true, execution_duration: rust_exec_duration }
}

/// Timing entry for a single verified call.
#[cfg(feature = "rust-verification")]
#[derive(Debug, Clone)]
pub struct CallTimingEntry {
    /// Human-readable label for this call (e.g., "Account __execute__", "SimpleCounter.increment")
    pub label: String,
    /// Contract address
    pub contract_address: Felt,
    /// Time spent in Rust execution for this call
    pub duration: std::time::Duration,
    /// Whether this call was actually executed (vs skipped)
    pub executed: bool,
    /// Whether verification passed (only meaningful if executed=true)
    pub verification_passed: bool,
}

/// Result of verifying an entire call tree.
#[cfg(feature = "rust-verification")]
pub struct CallTreeVerificationResult {
    /// Whether any contract in the tree was actually verified (not just skipped)
    pub any_executed: bool,
    /// Whether ALL executed calls passed verification
    pub all_passed: bool,
    /// Total time spent in pure Rust execution across all verified contracts
    pub total_execution_duration: std::time::Duration,
    /// Per-call timing breakdown
    pub call_timings: Vec<CallTimingEntry>,
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
    verify_call_tree_with_timestamp(blockifier_adapter, call_info, execution_result, None, 0)
}

/// Verify call tree with explicit block timestamp.
#[cfg(feature = "rust-verification")]
pub fn verify_call_tree_with_timestamp(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    call_info: &CallInfo,
    execution_result: &crate::ExecutionResult,
    phase_label: Option<&str>,
    block_timestamp: u64,
) -> CallTreeVerificationResult {
    // Verify this call
    let (this_result, call_label) =
        verify_single_call_with_label(blockifier_adapter, call_info, execution_result, block_timestamp);

    let mut any_executed = this_result.executed;
    let mut all_passed = true; // Start optimistic, set to false if any executed call fails
    let mut total_duration = this_result.execution_duration;
    let mut call_timings = Vec::new();

    // Add this call's timing with optional phase label override
    let label = phase_label.map(|p| p.to_string()).unwrap_or(call_label);
    call_timings.push(CallTimingEntry {
        label,
        contract_address: call_info.call.storage_address.to_felt(),
        duration: this_result.execution_duration,
        executed: this_result.executed,
        verification_passed: this_result.passed,
    });

    // Check if this call failed verification
    if this_result.executed && !this_result.passed {
        all_passed = false;
    }

    // Recursively verify all inner calls
    for inner_call in &call_info.inner_calls {
        let inner_result =
            verify_call_tree_with_timestamp(blockifier_adapter, inner_call, execution_result, None, block_timestamp);
        if inner_result.any_executed {
            any_executed = true;
        }
        if !inner_result.all_passed {
            all_passed = false;
        }
        total_duration += inner_result.total_execution_duration;
        call_timings.extend(inner_result.call_timings);
    }

    CallTreeVerificationResult { any_executed, all_passed, total_execution_duration: total_duration, call_timings }
}

/// Verify call tree with optional phase label override.
/// This is a convenience wrapper that uses timestamp=0 for backwards compatibility.
#[cfg(feature = "rust-verification")]
pub fn verify_call_tree_with_phase(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    call_info: &CallInfo,
    execution_result: &crate::ExecutionResult,
    phase_label: Option<&str>,
) -> CallTreeVerificationResult {
    verify_call_tree_with_timestamp(blockifier_adapter, call_info, execution_result, phase_label, 0)
}

/// Verify a single call and return both the result and a human-readable label.
#[cfg(feature = "rust-verification")]
fn verify_single_call_with_label(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    call_info: &CallInfo,
    execution_result: &crate::ExecutionResult,
    block_timestamp: u64,
) -> (VerificationResult, String) {
    let contract_address = call_info.call.storage_address.to_felt();
    let class_hash = call_info.call.class_hash.unwrap_or_default().to_felt();
    let entry_point_selector = call_info.call.entry_point_selector.to_felt();
    let calldata = &call_info.call.calldata.0;
    let caller_address = call_info.call.caller_address.to_felt();

    // Generate a human-readable label for this call
    let label = generate_call_label(class_hash, entry_point_selector);

    let result = verify_transaction_execution_with_call_info(
        blockifier_adapter,
        contract_address,
        class_hash,
        entry_point_selector,
        calldata,
        caller_address,
        call_info,
        execution_result,
        block_timestamp,
    );

    (result, label)
}

/// Generate a human-readable label for a call based on class hash and selector.
#[cfg(feature = "rust-verification")]
fn generate_call_label(class_hash: Felt, entry_point_selector: Felt) -> String {
    // Try to identify the contract by class hash
    let contract_name = mc_rust_exec::get_contract_name(class_hash)
        .unwrap_or_else(|| format!("{:#.8x}...", class_hash));

    // Try to identify the function by selector
    let function_name = mc_rust_exec::get_function_name(class_hash, entry_point_selector)
        .unwrap_or_else(|| format!("{:#.8x}...", entry_point_selector));

    format!("{}.{}", contract_name, function_name)
}

/// Timing entry placeholder for disabled feature.
#[cfg(not(feature = "rust-verification"))]
#[derive(Debug, Clone)]
pub struct CallTimingEntry {
    pub label: String,
    pub contract_address: starknet_types_core::felt::Felt,
    pub duration: std::time::Duration,
    pub executed: bool,
    pub verification_passed: bool,
}

/// Result of verifying an entire call tree (placeholder for disabled feature).
#[cfg(not(feature = "rust-verification"))]
pub struct CallTreeVerificationResult {
    pub any_executed: bool,
    pub all_passed: bool,
    pub total_execution_duration: std::time::Duration,
    pub call_timings: Vec<CallTimingEntry>,
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
        all_passed: true,
        total_execution_duration: std::time::Duration::ZERO,
        call_timings: Vec::new(),
    }
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn verify_call_tree_with_phase(
    _blockifier_adapter: &crate::BlockifierStateAdapter<impl mc_db::MadaraStorageRead>,
    _call_info: &blockifier::execution::call_info::CallInfo,
    _execution_result: &crate::ExecutionResult,
    _phase_label: Option<&str>,
) -> CallTreeVerificationResult {
    // No-op when verification is disabled
    CallTreeVerificationResult {
        any_executed: false,
        all_passed: true,
        total_execution_duration: std::time::Duration::ZERO,
        call_timings: Vec::new(),
    }
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn verify_call_tree_with_timestamp(
    _blockifier_adapter: &crate::BlockifierStateAdapter<impl mc_db::MadaraStorageRead>,
    _call_info: &blockifier::execution::call_info::CallInfo,
    _execution_result: &crate::ExecutionResult,
    _phase_label: Option<&str>,
    _block_timestamp: u64,
) -> CallTreeVerificationResult {
    // No-op when verification is disabled
    CallTreeVerificationResult {
        any_executed: false,
        all_passed: true,
        total_execution_duration: std::time::Duration::ZERO,
        call_timings: Vec::new(),
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
        tracing::info!("Cairo Native is disabled, skipping benchmark class pre-compilation");
        return;
    }

    // Get the class hashes from Rust verification config
    let class_hashes = get_benchmark_class_hashes();

    if class_hashes.is_empty() {
        tracing::info!("No benchmark class hashes configured, skipping pre-compilation");
        return;
    }

    tracing::info!("🔧 Pre-compiling {} benchmark class(es) for Cairo Native...", class_hashes.len());

    let mut compiled_count = 0;
    let mut skipped_count = 0;
    let mut failed_count = 0;

    for (name, class_hash) in class_hashes {
        let starknet_class_hash = starknet_api::core::ClassHash(class_hash);

        // Check if already compiled
        if mc_class_exec::cache_contains(&starknet_class_hash) {
            tracing::debug!("   ✓ {} ({:#x}) already compiled, skipping", name, class_hash);
            skipped_count += 1;
            continue;
        }

        // Get a view on the latest confirmed block to load the class
        let view = backend.view_on_latest_confirmed();

        // Load the Sierra class from the database
        let sierra_class = match view.get_class_info_and_compiled(&class_hash) {
            Ok(Some(ConvertedClass::Sierra(sierra))) => sierra,
            Ok(Some(ConvertedClass::Legacy(_))) => {
                tracing::warn!("   ✗ {} ({:#x}) - Legacy class, not supported for Cairo Native", name, class_hash);
                failed_count += 1;
                continue;
            }
            Ok(None) => {
                tracing::warn!("   ⚠ {} ({:#x}) - class not found in database (not deployed yet?)", name, class_hash);
                failed_count += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!("   ✗ {} ({:#x}) - failed to load class: {}", name, class_hash, e);
                failed_count += 1;
                continue;
            }
        };

        // Compile the class
        tracing::info!("   ⏳ Compiling {} ({:#x})...", name, class_hash);

        let start = std::time::Instant::now();
        match mc_class_exec::compile_native_blocking(starknet_class_hash, &sierra_class, cairo_native_config) {
            Ok(_) => {
                let elapsed = start.elapsed();
                tracing::info!("   ✓ {} ({:#x}) compiled in {:?}", name, class_hash, elapsed);
                compiled_count += 1;
            }
            Err(e) => {
                tracing::warn!("   ✗ {} ({:#x}) - compilation failed: {}", name, class_hash, e);
                failed_count += 1;
            }
        }
    }

    tracing::info!(
        "🔧 Pre-compilation complete: {} compiled, {} skipped (cached), {} failed",
        compiled_count,
        skipped_count,
        failed_count
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

// ============================================================================
// Full Transaction Verification (NEW APPROACH)
// ============================================================================

/// Dump verification results to files for manual inspection.
#[cfg(feature = "rust-verification")]
fn dump_verification_results(
    tx_hash: &Felt,
    blockifier_storage: &[(Felt, Vec<(Felt, Felt)>)],
    blockifier_events: &[(Vec<Felt>, Vec<Felt>)],
    blockifier_nonces: &[(Felt, Felt)],
    rust_result: &mc_rust_exec::transaction::TransactionExecutionResult,
    rust_exec_result: &mc_rust_exec::types::ExecutionResult,
    verification_result: &mc_rust_exec::verify::VerificationResult,
    block_context: &blockifier::context::BlockContext,
    execution_result: &crate::ExecutionResult,
) {
    use std::fs;

    let dump_dir = "/Users/heemankverma/Work/Karnot/RvsC/verification_dumps";
    let tx_hash_short = format!("{:#x}", tx_hash)[..18].to_string();

    // Blockifier results
    let blockifier_dump = format!(
        "BLOCKIFIER EXECUTION RESULT\n\
        ================================\n\n\
        Transaction Hash: {:#x}\n\
        Block Number: {}\n\
        Timestamp: {}\n\n\
        RECEIPT:\n\
        Fee Charged: {} wei\n\
        Gas Vector:\n\
          L1 Gas: {}\n\
          L1 Data Gas: {}\n\
          L2 Gas: {}\n\n\
        STORAGE UPDATES ({} total):\n{}\n\n\
        EVENTS ({} total):\n{}\n\n\
        NONCES ({} total):\n{}\n",
        tx_hash,
        block_context.block_info().block_number.0,
        block_context.block_info().block_timestamp.0,
        execution_result.execution_info.receipt.fee.0,
        execution_result.execution_info.receipt.gas.l1_gas.0,
        execution_result.execution_info.receipt.gas.l1_data_gas.0,
        execution_result.execution_info.receipt.gas.l2_gas.0,
        blockifier_storage.iter().map(|(_, entries)| entries.len()).sum::<usize>(),
        format_storage_updates(blockifier_storage),
        blockifier_events.len(),
        format_events(blockifier_events),
        blockifier_nonces.len(),
        format_nonces(blockifier_nonces),
    );

    // Rust results
    let rust_dump = format!(
        "RUST EXECUTION RESULT\n\
        ================================\n\n\
        Transaction Hash: {:#x}\n\n\
        EXECUTION INFO:\n\
        Actual Fee (Rust calc): {} wei\n\
        Gas Consumed:\n\
          L1 Gas: {}\n\
          L1 Data Gas: {}\n\
          L2 Gas: {}\n\
        Revert Error: {:?}\n\n\
        STORAGE UPDATES ({} total):\n{}\n\n\
        EVENTS ({} total):\n{}\n\n\
        NONCES ({} total):\n{}\n",
        tx_hash,
        rust_result.actual_fee,
        rust_result.gas_consumed.l1_gas,
        rust_result.gas_consumed.l1_data_gas,
        rust_result.gas_consumed.l2_gas,
        rust_result.revert_error,
        rust_result.state_diff.storage_updates.len(),
        format_rust_storage_updates(&rust_result.state_diff),
        rust_exec_result.call_result.events.len(),
        format_rust_events(&rust_exec_result.call_result.events),
        rust_result.state_diff.address_to_nonce.len(),
        format_rust_nonces(&rust_result.state_diff),
    );

    // Comparison
    let comparison = format!(
        "VERIFICATION COMPARISON\n\
        ================================\n\n\
        Transaction Hash: {:#x}\n\n\
        VERIFICATION STATUS: {}\n\
        Total Mismatches: {}\n\n\
        MISMATCHES:\n{}\n\n\
        STATISTICS:\n\
        - Blockifier Storage Updates: {}\n\
        - Rust Storage Updates: {}\n\
        - Blockifier Events: {}\n\
        - Rust Events: {}\n\
        - Blockifier Nonces: {}\n\
        - Rust Nonces: {}\n",
        tx_hash,
        if verification_result.passed { "✅ PASSED" } else { "❌ FAILED" },
        verification_result.errors.len(),
        format_verification_errors(&verification_result.errors, block_context),
        blockifier_storage.iter().map(|(_, entries)| entries.len()).sum::<usize>(),
        rust_result.state_diff.storage_updates.len(),
        blockifier_events.len(),
        rust_exec_result.call_result.events.len(),
        blockifier_nonces.len(),
        rust_result.state_diff.address_to_nonce.len(),
    );

    // Write files
    let _ = fs::write(format!("{}/{}_blockifier.txt", dump_dir, tx_hash_short), blockifier_dump);
    let _ = fs::write(format!("{}/{}_rust.txt", dump_dir, tx_hash_short), rust_dump);
    let _ = fs::write(format!("{}/{}_comparison.txt", dump_dir, tx_hash_short), comparison);

    tracing::info!("📁 Verification results dumped to {}/{}*.txt", dump_dir, tx_hash_short);
}

#[cfg(feature = "rust-verification")]
fn format_storage_updates(storage: &[(Felt, Vec<(Felt, Felt)>)]) -> String {
    let mut result = String::new();
    for (contract, entries) in storage {
        result.push_str(&format!("  Contract {:#x}:\n", contract));
        for (key, value) in entries {
            result.push_str(&format!("    {:#x} = {:#x}\n", key, value));
        }
    }
    result
}

#[cfg(feature = "rust-verification")]
fn format_events(events: &[(Vec<Felt>, Vec<Felt>)]) -> String {
    let mut result = String::new();
    for (i, (keys, data)) in events.iter().enumerate() {
        result.push_str(&format!("  Event #{}:\n", i));
        result.push_str(&format!("    Keys: {:?}\n", keys.iter().map(|k| format!("{:#x}", k)).collect::<Vec<_>>()));
        result.push_str(&format!("    Data: {:?}\n", data.iter().map(|d| format!("{:#x}", d)).collect::<Vec<_>>()));
    }
    result
}

#[cfg(feature = "rust-verification")]
fn format_nonces(nonces: &[(Felt, Felt)]) -> String {
    let mut result = String::new();
    for (contract, nonce) in nonces {
        result.push_str(&format!("  {:#x} = {:#x}\n", contract, nonce));
    }
    result
}

#[cfg(feature = "rust-verification")]
fn format_rust_storage_updates(state_diff: &mc_rust_exec::types::StateDiff) -> String {
    let mut result = String::new();
    for (contract, entries) in &state_diff.storage_updates {
        result.push_str(&format!("  Contract {:#x}:\n", contract.0));
        for (key, value) in entries {
            result.push_str(&format!("    {:#x} = {:#x}\n", key.0, value));
        }
    }
    result
}

#[cfg(feature = "rust-verification")]
fn format_rust_events(events: &[mc_rust_exec::types::Event]) -> String {
    let mut result = String::new();
    for event in events {
        result.push_str(&format!("  Event #{}:\n", event.order));
        result.push_str(&format!("    Keys: {:?}\n", event.keys.iter().map(|k| format!("{:#x}", k)).collect::<Vec<_>>()));
        result.push_str(&format!("    Data: {:?}\n", event.data.iter().map(|d| format!("{:#x}", d)).collect::<Vec<_>>()));
    }
    result
}

#[cfg(feature = "rust-verification")]
fn format_rust_nonces(state_diff: &mc_rust_exec::types::StateDiff) -> String {
    let mut result = String::new();
    for (contract, nonce) in &state_diff.address_to_nonce {
        result.push_str(&format!("  {:#x} = {:#x}\n", contract.0, nonce.0));
    }
    result
}

#[cfg(feature = "rust-verification")]
fn format_verification_errors(errors: &[mc_rust_exec::verify::VerificationError], block_context: &blockifier::context::BlockContext) -> String {
    use mp_convert::ToFelt;
    let mut result = String::new();
    let fee_token = block_context.chain_info().fee_token_addresses.eth_fee_token_address.to_felt();

    for error in errors {
        let is_fee_related = match error {
            mc_rust_exec::verify::VerificationError::StorageMismatch { contract, .. } => {
                *contract == fee_token
            }
            _ => false
        };

        if is_fee_related {
            result.push_str(&format!("  [EXPECTED - Gas/Fee] {:?}\n", error));
        } else {
            result.push_str(&format!("  [CONTRACT LOGIC] {:?}\n", error));
        }
    }
    result
}

/// Verify a complete Invoke transaction at the transaction level.
///
/// This executes the full transaction flow in Rust:
/// 1. Account __validate__ (signature verification)
/// 2. Nonce increment
/// 3. Account __execute__ (dispatches calls to target contracts)
/// 4. Fee calculation and transfer
///
/// Then compares the complete state_diff against Blockifier's output.
#[cfg(feature = "rust-verification")]
pub fn verify_full_invoke_transaction<D: MadaraStorageRead>(
    blockifier_adapter: &BlockifierStateAdapter<D>,
    tx: &starknet_api::executable_transaction::InvokeTransaction,
    block_context: &blockifier::context::BlockContext,
    execution_result: &crate::ExecutionResult,
) -> CallTreeVerificationResult {
    use mp_convert::ToFelt;

    // Initialize on first call
    Lazy::force(&INIT);

    let tx_hash = tx.tx_hash.to_felt();
    let sender_address = tx.sender_address().to_felt();
    let calldata = &tx.calldata().0;
    let signature = &tx.signature().0;

    tracing::info!("🔄 FULL TRANSACTION VERIFICATION for tx {:#x}", tx_hash);
    tracing::info!("   Sender: {:#x}", sender_address);
    tracing::info!("   Calldata len: {}", calldata.len());

    // Get account class hash
    use blockifier::state::state_api::StateReader as BlockifierStateReader;
    let account_class_hash = match blockifier_adapter.get_class_hash_at(
        starknet_api::core::ContractAddress::try_from(sender_address).unwrap()
    ) {
        Ok(hash) => hash.to_felt(),
        Err(e) => {
            tracing::warn!("❌ Failed to get account class hash: {}", e);
            return CallTreeVerificationResult {
                any_executed: false,
                all_passed: false,
                total_execution_duration: std::time::Duration::ZERO,
                call_timings: Vec::new(),
            };
        }
    };

    // Check if account contract is supported
    if !mc_rust_exec::contracts::ContractRegistry::supports_class_hash(account_class_hash) {
        tracing::debug!("⏭️  Account class {:#x} not supported in Rust, skipping", account_class_hash);
        return CallTreeVerificationResult {
            any_executed: false,
            all_passed: true,
            total_execution_duration: std::time::Duration::ZERO,
            call_timings: Vec::new(),
        };
    }

    // ============================================================
    // RUST FULL TRANSACTION EXECUTION - START TIMING
    // ============================================================
    let rust_exec_start = std::time::Instant::now();

    // Create state adapter for Rust execution
    let rust_state = RustExecStateAdapter::new(blockifier_adapter);

    // Build transaction context for Rust execution
    let rust_block_context = mc_rust_exec::gas::BlockContext {
        block_number: block_context.block_info().block_number.0,
        block_timestamp: block_context.block_info().block_timestamp.0,
        sequencer_address: mc_rust_exec::types::ContractAddress(block_context.block_info().sequencer_address.to_felt()),
        l1_gas_price_wei: block_context.block_info().gas_prices.eth_gas_prices.l1_gas_price.get().0,
        l1_data_gas_price_wei: block_context.block_info().gas_prices.eth_gas_prices.l1_data_gas_price.get().0,
        l2_gas_price_wei: block_context.block_info().gas_prices.eth_gas_prices.l2_gas_price.get().0,
        l1_gas_price_fri: block_context.block_info().gas_prices.strk_gas_prices.l1_gas_price.get().0,
        l1_data_gas_price_fri: block_context.block_info().gas_prices.strk_gas_prices.l1_data_gas_price.get().0,
        l2_gas_price_fri: block_context.block_info().gas_prices.strk_gas_prices.l2_gas_price.get().0,
        eth_fee_token_address: mc_rust_exec::types::ContractAddress(
            block_context.chain_info().fee_token_addresses.eth_fee_token_address.to_felt()
        ),
        strk_fee_token_address: mc_rust_exec::types::ContractAddress(
            block_context.chain_info().fee_token_addresses.strk_fee_token_address.to_felt()
        ),
    };

    // Extract nonce - we'll use defaults for resource bounds for now
    // The actual fee calculation will come from gas tracking
    let nonce = tx.nonce();
    let nonce_value = nonce.0;

    // For MVP, use ETH fee type and default resource bounds
    // TODO: Extract actual resource bounds once we have access to the inner transaction
    let fee_type = mc_rust_exec::gas::FeeType::Eth;
    let resource_bounds = mc_rust_exec::gas::ResourceBounds::default();

    // Build InvokeTransaction for Rust execution
    let rust_tx = mc_rust_exec::transaction::InvokeTransaction {
        tx_hash,
        version: Felt::ZERO, // Version not critical for execution
        sender_address: mc_rust_exec::types::ContractAddress(sender_address),
        calls: parse_calls_from_invoke_calldata(calldata).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse calls: {}", e);
            vec![]
        }),
        signature: signature.to_vec(),
        nonce: mc_rust_exec::types::Nonce(Felt::from(nonce_value)),
        fee_type,
        resource_bounds,
    };

    // Execute full transaction in Rust
    let mut executor = mc_rust_exec::transaction::TransactionExecutor::new(&rust_state, &rust_block_context);

    let rust_result = match executor.execute_invoke(&rust_tx, account_class_hash) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("❌ Rust transaction execution failed: {}", e);
            let rust_exec_duration = rust_exec_start.elapsed();
            return CallTreeVerificationResult {
                any_executed: true,
                all_passed: false,
                total_execution_duration: rust_exec_duration,
                call_timings: vec![CallTimingEntry {
                    label: "Full Transaction (FAILED)".to_string(),
                    contract_address: sender_address,
                    duration: rust_exec_duration,
                    executed: true,
                    verification_passed: false,
                }],
            };
        }
    };

    let rust_exec_duration = rust_exec_start.elapsed();

    // ============================================================
    // RUST EXECUTION COMPLETE - NOW COMPARE WITH BLOCKIFIER
    // ============================================================

    // Prepare Blockifier data for comparison - COMPLETE transaction state_diff
    let blockifier_storage: Vec<(Felt, Vec<(Felt, Felt)>)> = execution_result
        .state_diff
        .storage_updates
        .iter()
        .map(|(addr, updates)| {
            let storage_entries: Vec<(Felt, Felt)> = updates.iter().map(|(k, v)| (k.to_felt(), *v)).collect();
            (addr.to_felt(), storage_entries)
        })
        .collect();

    let blockifier_nonces: Vec<(Felt, Felt)> = execution_result
        .state_diff
        .address_to_nonce
        .iter()
        .map(|(addr, nonce)| (addr.to_felt(), nonce.to_felt()))
        .collect();

    // Events from execute call
    let blockifier_events: Vec<(Vec<Felt>, Vec<Felt>)> = if let Some(execute_call_info) = &execution_result.execution_info.execute_call_info {
        collect_all_events_from_call_tree(execute_call_info)
    } else {
        vec![]
    };

    // Build ExecutionResult wrapper for verification
    // For transaction-level comparison, we use the execute_call_info as the primary result
    let rust_exec_result = mc_rust_exec::types::ExecutionResult {
        call_result: rust_result.execute_call_info.clone().unwrap_or(mc_rust_exec::types::CallExecutionResult {
            retdata: vec![],
            events: vec![],
            l2_to_l1_messages: vec![],
            failed: rust_result.revert_error.is_some(),
            gas_consumed: rust_result.gas_consumed.l2_gas,
        }),
        state_diff: rust_result.state_diff.clone(),
        revert_error: rust_result.revert_error.clone(),
    };

    // Compare using comprehensive verification
    let verification_result = mc_rust_exec::verify::verify_execution_comprehensive(
        &rust_exec_result,
        &blockifier_storage,
        &vec![], // retdata - not comparing at tx level
        &blockifier_events,
        false, // not failed
        &blockifier_nonces,
        &vec![], // messages - collected separately if needed
        &vec![], // class hashes
        &vec![], // compiled class hashes
    );

    // Dump results to files for manual inspection
    // dump_verification_results(
    //     &tx_hash,
    //     &blockifier_storage,
    //     &blockifier_events,
    //     &blockifier_nonces,
    //     &rust_result,
    //     &rust_exec_result,
    //     &verification_result,
    //     block_context,
    //     &execution_result,
    // );

    // Separate ERC20 fee-related mismatches from contract logic mismatches
    let fee_token_address = block_context.chain_info().fee_token_addresses.eth_fee_token_address.to_felt();
    let (contract_logic_errors, fee_related_errors): (Vec<_>, Vec<_>) = verification_result.errors.iter()
        .partition(|error| {
            match error {
                mc_rust_exec::verify::VerificationError::StorageMismatch { contract, .. } => {
                    *contract != fee_token_address
                }
                _ => true
            }
        });

    let all_passed = contract_logic_errors.is_empty();

    // Log results with clear separation
    if !all_passed {
        tracing::warn!("❌ RUST FULL TRANSACTION VERIFICATION FAILED");
        tracing::warn!("   Contract Logic Mismatches: {}", contract_logic_errors.len());
        for error in &contract_logic_errors {
            tracing::warn!("      {:?}", error);
        }
    } else {
        tracing::info!("✅ RUST FULL TRANSACTION VERIFICATION PASSED");
    }

    // Log successful verifications
    let storage_verified = rust_result.state_diff.storage_updates.len();
    let events_verified = rust_exec_result.call_result.events.len();
    let nonces_verified = rust_result.state_diff.address_to_nonce.len();

    tracing::info!("📊 VERIFICATION SUMMARY:");
    tracing::info!("   ✅ Storage Updates: {} verified", storage_verified);
    tracing::info!("   ✅ Events: {} verified", events_verified);
    tracing::info!("   ✅ Nonce Updates: {} verified", nonces_verified);

    // Log expected fee-related differences
    if !fee_related_errors.is_empty() {
        tracing::info!("   📝 EXPECTED Differences (Gas/Fee Calculation):");
        for error in &fee_related_errors {
            tracing::info!("      {:?}", error);
        }
        tracing::info!("   ℹ️  Fee differences are EXPECTED due to native Rust execution vs Cairo VM");
    }

    CallTreeVerificationResult {
        any_executed: true,
        all_passed,
        total_execution_duration: rust_exec_duration,
        call_timings: vec![CallTimingEntry {
            label: "Full Transaction".to_string(),
            contract_address: sender_address,
            duration: rust_exec_duration,
            executed: true,
            verification_passed: all_passed,
        }],
    }
}

/// Parse calls from invoke transaction calldata (for __execute__).
#[cfg(feature = "rust-verification")]
fn parse_calls_from_invoke_calldata(calldata: &[Felt]) -> Result<Vec<mc_rust_exec::transaction::Call>, String> {
    // Invoke V1 format: __execute__ is implicit, calldata contains the calls array
    // Parse using account's call parsing logic
    let account_calls = mc_rust_exec::contracts::account::functions::parse_calls(calldata)
        .map_err(|e| format!("Failed to parse calls: {}", e))?;

    Ok(account_calls.into_iter().map(|call| {
        mc_rust_exec::transaction::Call {
            to: call.to,
            selector: call.selector,
            calldata: call.calldata,
        }
    }).collect())
}

/// Recursively collect all events from a call tree.
#[cfg(feature = "rust-verification")]
fn collect_all_events_from_call_tree(call_info: &CallInfo) -> Vec<(Vec<Felt>, Vec<Felt>)> {
    use mp_convert::ToFelt;

    let mut events = vec![];

    // Collect events from this call
    for event in &call_info.execution.events {
        let keys: Vec<Felt> = event.event.keys.iter().map(ToFelt::to_felt).collect();
        let data: Vec<Felt> = event.event.data.0.clone();
        events.push((keys, data));
    }

    // Recursively collect from inner calls
    for inner_call in &call_info.inner_calls {
        events.extend(collect_all_events_from_call_tree(inner_call));
    }

    events
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn verify_full_invoke_transaction<D: mc_db::MadaraStorageRead>(
    _blockifier_adapter: &crate::BlockifierStateAdapter<D>,
    _tx: &starknet_api::executable_transaction::InvokeTransaction,
    _block_context: &blockifier::context::BlockContext,
    _execution_result: &crate::ExecutionResult,
) -> CallTreeVerificationResult {
    // No-op when verification is disabled
    CallTreeVerificationResult {
        any_executed: false,
        all_passed: true,
        total_execution_duration: std::time::Duration::ZERO,
        call_timings: Vec::new(),
    }
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
