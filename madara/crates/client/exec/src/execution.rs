use crate::metrics::TxExecutionTimer;
use crate::{Error, ExecutionContext, ExecutionResult, TxExecError};
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::TransactionalState;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{HasRelatedFeeType, TransactionInfoCreatorInner};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::ExecutableTransaction;
use mc_db::MadaraStorageRead;
use mp_convert::ToFelt;
use mp_receipt::RevertErrorExt;
use starknet_api::block::FeeType;
use starknet_api::contract_class::ContractClass;
use starknet_api::core::{ClassHash, ContractAddress, Nonce};
use starknet_api::executable_transaction::{AccountTransaction as ApiAccountTransaction, TransactionType};
use starknet_api::transaction::fields::{GasVectorComputationMode, Tip};
use starknet_api::transaction::{TransactionHash, TransactionVersion};
use std::time::Instant;

/// Log trace transaction stats to file for analysis.
/// Format: <txn_hash>:rust_exec_time:cairo_exec_time:was_it_a_proper_match(true/false)
#[cfg(feature = "rust-verification")]
fn log_trace_stats(
    tx_hash: starknet_types_core::felt::Felt,
    rust_duration: std::time::Duration,
    blockifier_duration: std::time::Duration,
    all_passed: bool,
) {
    use std::io::Write;

    let log_path = "/Users/heemankverma/Work/Karnot/RvsC/stats/tracecalls.log";

    // Create directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(log_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Format durations as microseconds for precision
    let rust_micros = rust_duration.as_micros();
    let blockifier_micros = blockifier_duration.as_micros();

    let log_line = format!(
        "{:#x}:{}us:{}us:{}\n",
        tx_hash,
        rust_micros,
        blockifier_micros,
        all_passed
    );

    // Append to log file
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = file.write_all(log_line.as_bytes());
    } else {
        tracing::warn!("Failed to write trace stats to {}", log_path);
    }
}

impl<D: MadaraStorageRead> ExecutionContext<D> {
    /// Execute transactions. The returned `ExecutionResult`s are the results of the `transactions_to_trace`. The results of `transactions_before` are discarded.
    /// This function is useful for tracing trasaction execution, by reexecuting the block.
    pub fn execute_transactions(
        &mut self,
        transactions_before: impl IntoIterator<Item = Transaction>,
        transactions_to_trace: impl IntoIterator<Item = Transaction>,
    ) -> Result<Vec<ExecutionResult>, Error> {
        let mut executed_prev = 0;
        for (index, tx) in transactions_before.into_iter().enumerate() {
            let hash = tx.tx_hash();
            let tx_type = tx.tx_type();
            tracing::debug!("executing {:#x}", hash.to_felt());
            let timer = TxExecutionTimer::new();
            tx.execute(&mut self.state, &self.block_context).map_err(|err| TxExecError {
                view: format!("{}", self.view()),
                hash,
                index,
                err,
            })?;
            timer.finish(tx_type);
            executed_prev += 1;
        }

        transactions_to_trace
            .into_iter()
            .enumerate()
            .map(|(index, tx): (_, Transaction)| {
                let hash = tx.tx_hash();
                tracing::debug!("executing {:#x} (trace)", hash.to_felt());
                let tx_type = tx.tx_type();
                let fee_type = tx.fee_type();

                // We need to estimate gas too.
                let minimal_l1_gas = match &tx {
                    Transaction::Account(tx) => {
                        Some(estimate_minimal_gas_vector(&self.block_context, tx, &GasVectorComputationMode::All))
                    }
                    Transaction::L1Handler(_) => None, // There is no minimal_l1_gas field for L1 handler transactions.
                };

                let view = self.view().clone();
                let make_reexec_error =
                    |err| TxExecError { view: format!("{view}"), hash, index: executed_prev + index, err };

                // Clone view and block_number BEFORE the mutable borrow for Rust verification.
                // This gives us a separate adapter that reads from DB directly (pre-execution state).
                let pre_exec_view = self.view().clone();
                let pre_exec_block_number = self.blockifier_state_adapter().block_number;

                // Debug: log what state we're using for verification
                tracing::info!(
                    "🔬 VERIFICATION STATE DEBUG: pre_exec_view={}, block_number={}, executed_prev={}",
                    pre_exec_view,
                    pre_exec_block_number,
                    executed_prev
                );

                // HACK: Skip Rust verification if there were previous transactions in this block.
                // The cloned view reads from DB (block N-1) but CachedState has previous txs' changes.
                // This causes state mismatches for txs after the first one in a block.
                // TODO: Properly handle this by reading from CachedState or snapshotting state.
                let skip_verification = executed_prev > 0;
                if skip_verification {
                    tracing::info!(
                        "⚠️  Skipping Rust verification: {} previous tx(s) modified state in CachedState",
                        executed_prev
                    );
                }

                let mut transactional_state = TransactionalState::create_transactional(&mut self.state);

                // ============================================================
                // BLOCKIFIER (Cairo VM) EXECUTION - START TIMING
                // ============================================================
                let blockifier_start = Instant::now();

                // NB: We use execute_raw because execute already does transaactional state.
                let timer = TxExecutionTimer::new();
                let mut execution_info =
                    tx.execute_raw(&mut transactional_state, &self.block_context, false).map_err(make_reexec_error)?;
                timer.finish(tx_type);

                execution_info.revert_error = execution_info.revert_error.take().map(|e| e.format_for_receipt());

                let state_diff = transactional_state
                    .to_state_diff()
                    .map_err(TransactionExecutionError::StateError)
                    .map_err(make_reexec_error)?;

                // ============================================================
                // BLOCKIFIER (Cairo VM) EXECUTION - END TIMING
                // ============================================================
                let blockifier_duration = blockifier_start.elapsed();

                let gas_vector_computation_mode = match &tx {
                    Transaction::Account(account_tx) => account_tx.tx.create_tx_info(account_tx.execution_flags.only_query).gas_mode(),
                    Transaction::L1Handler(_) => GasVectorComputationMode::NoL2Gas,
                };

                let result = ExecutionResult {
                    hash,
                    tx_type,
                    fee_type,
                    minimal_l1_gas,
                    execution_info,
                    gas_vector_computation_mode,
                    state_diff: state_diff.state_maps.into(),
                };

                // Commit transactional state to make changes available for next transaction
                transactional_state.commit();

                // Rust verification - Use FULL TRANSACTION verification for Invoke transactions
                // This replicates the entire transaction flow: validate → nonce → execute → fee
                let verification_result = if skip_verification {
                    // Skip verification - state is polluted by previous transactions
                    crate::rust_exec_integration::CallTreeVerificationResult {
                        any_executed: false,
                        all_passed: true,
                        total_execution_duration: std::time::Duration::ZERO,
                        call_timings: Vec::new(),
                    }
                } else {
                    let verification_adapter =
                        crate::BlockifierStateAdapter::new(pre_exec_view, pre_exec_block_number);

                    // Try full transaction verification for Invoke transactions
                    match &tx {
                        Transaction::Account(account_tx) => {
                            match &account_tx.tx {
                                starknet_api::executable_transaction::AccountTransaction::Invoke(invoke_tx) => {
                                    // Use full transaction verification
                                    tracing::info!("🔄 Using FULL TRANSACTION verification for Invoke");
                                    crate::rust_exec_integration::verify_full_invoke_transaction(
                                        &verification_adapter,
                                        invoke_tx,
                                        &self.block_context,
                                        &result,
                                    )
                                }
                                _ => {
                                    // Fall back to call-tree for other transaction types
                                    call_tree_verification_fallback(&verification_adapter, &result)
                                }
                            }
                        }
                        _ => {
                            // Fall back to call-tree for L1Handler
                            call_tree_verification_fallback(&verification_adapter, &result)
                        }
                    }
                };

                // Log timing comparison if Rust execution was performed
                if verification_result.any_executed {
                    let rust_duration = verification_result.total_execution_duration;
                    let speedup = if rust_duration.as_nanos() > 0 {
                        blockifier_duration.as_nanos() as f64 / rust_duration.as_nanos() as f64
                    } else {
                        f64::INFINITY
                    };

                    // Note: Can't easily determine actual execution mode (Native vs VM) at tx level
                    // since different calls may use different modes based on TrackedResource.
                    // Use "native enabled" to indicate config, not actual execution.
                    let blockifier_mode = if self.is_cairo_native_enabled() { "Cairo Native" } else { "VM only" };

                    tracing::info!("⏱️  TIMING COMPARISON for tx {:#x}:", hash.to_felt());
                    tracing::info!("   🐢 Blockifier ({}): {:?}", blockifier_mode, blockifier_duration);
                    tracing::info!("   🚀 Rust Execution:       {:?}", rust_duration);
                    tracing::info!(
                        "   📊 Speedup: {:.2}x {}",
                        speedup,
                        if speedup > 1.0 { "(Rust faster)" } else { "(Blockifier faster)" }
                    );

                    // Log trace stats to file
                    log_trace_stats(
                        hash.to_felt(),
                        rust_duration,
                        blockifier_duration,
                        verification_result.all_passed,
                    );

                    // Log per-phase timing breakdown
                    if !verification_result.call_timings.is_empty() {
                        tracing::info!("   📋 RUST EXECUTION BREAKDOWN:");
                        for timing in &verification_result.call_timings {
                            if timing.executed {
                                tracing::info!(
                                    "      {} ({:#.8x}...): {:?}",
                                    timing.label,
                                    timing.contract_address,
                                    timing.duration
                                );
                            } else {
                                tracing::info!(
                                    "      {} ({:#.8x}...): skipped (not implemented)",
                                    timing.label,
                                    timing.contract_address
                                );
                            }
                        }
                    }

                    // Comprehensive verification summary
                    let all_executed: Vec<_> = verification_result.call_timings.iter().filter(|t| t.executed).collect();
                    if !all_executed.is_empty() {
                        // Determine overall status
                        let status_icon = if verification_result.all_passed { "✅" } else { "❌" };
                        let status_text = if verification_result.all_passed { "PASSED" } else { "FAILED" };

                        if verification_result.all_passed {
                            tracing::info!("   {} RUST VERIFICATION REPORT:", status_icon);
                        } else {
                            tracing::warn!("   {} RUST VERIFICATION REPORT:", status_icon);
                        }

                        // Count passed vs failed
                        let passed_count = all_executed.iter().filter(|t| t.verification_passed).count();
                        let failed_count = all_executed.len() - passed_count;

                        if verification_result.all_passed {
                            tracing::info!("      📊 Calls Verified: {} (all passed)", all_executed.len());
                        } else {
                            tracing::warn!("      📊 Calls Verified: {} ({} passed, {} FAILED)", all_executed.len(), passed_count, failed_count);

                            // List which calls failed
                            for timing in &all_executed {
                                if !timing.verification_passed {
                                    tracing::warn!("         ❌ FAILED: {} ({:#.8x}...)", timing.label, timing.contract_address);
                                }
                            }
                        }

                        // Count what was checked from the transaction state diff
                        let storage_contracts = result.state_diff.storage_updates.len();
                        let total_storage_writes: usize = result.state_diff.storage_updates.values().map(|v| v.len()).sum();
                        let nonce_updates = result.state_diff.address_to_nonce.len();
                        let class_hash_updates = result.state_diff.address_to_class_hash.len();
                        let compiled_class_updates = result.state_diff.class_hash_to_compiled_class_hash.len();

                        // Count events and messages from all executed calls
                        let mut total_events = 0;
                        let mut total_messages = 0;

                        if let Some(execute_call_info) = &result.execution_info.execute_call_info {
                            total_events += count_events_in_call_tree(execute_call_info);
                            total_messages += count_messages_in_call_tree(execute_call_info);
                        }
                        if let Some(fee_transfer_call_info) = &result.execution_info.fee_transfer_call_info {
                            total_events += count_events_in_call_tree(fee_transfer_call_info);
                            total_messages += count_messages_in_call_tree(fee_transfer_call_info);
                        }

                        // Log details with appropriate log level
                        if verification_result.all_passed {
                            if total_storage_writes > 0 {
                                tracing::info!("      💾 Storage Writes: {} write(s) across {} contract(s)",
                                    total_storage_writes, storage_contracts);
                            }
                            if nonce_updates > 0 {
                                tracing::info!("      🔢 Nonce Updates: {} contract(s)", nonce_updates);
                            }
                            if total_events > 0 {
                                tracing::info!("      📢 Events: {}", total_events);
                            }
                            if total_messages > 0 {
                                tracing::info!("      ✉️  L2→L1 Messages: {}", total_messages);
                            }
                            if class_hash_updates > 0 {
                                tracing::info!("      🏗️  Class Hash Updates: {} contract(s)", class_hash_updates);
                            }
                            if compiled_class_updates > 0 {
                                tracing::info!("      📦 Compiled Class Hashes: {}", compiled_class_updates);
                            }
                        } else {
                            if total_storage_writes > 0 {
                                tracing::warn!("      💾 Storage Writes: {} write(s) across {} contract(s)",
                                    total_storage_writes, storage_contracts);
                            }
                            if nonce_updates > 0 {
                                tracing::warn!("      🔢 Nonce Updates: {} contract(s)", nonce_updates);
                            }
                            if total_events > 0 {
                                tracing::warn!("      📢 Events: {}", total_events);
                            }
                            if total_messages > 0 {
                                tracing::warn!("      ✉️  L2→L1 Messages: {}", total_messages);
                            }
                            if class_hash_updates > 0 {
                                tracing::warn!("      🏗️  Class Hash Updates: {} contract(s)", class_hash_updates);
                            }
                            if compiled_class_updates > 0 {
                                tracing::warn!("      📦 Compiled Class Hashes: {}", compiled_class_updates);
                            }
                        }

                        if verification_result.all_passed {
                            tracing::info!("      {} VERIFICATION {}: Rust output matches Blockifier", status_icon, status_text);
                        } else {
                            tracing::warn!("      {} VERIFICATION {}: Rust output does NOT match Blockifier (see detailed errors above)", status_icon, status_text);
                        }
                    }

                    // Full state diff logging (commented out for cleaner output)
                    // Uncomment below for debugging:
                    // tracing::info!("📋 FULL TRANSACTION STATE DIFF (from Blockifier):");
                    // if !result.state_diff.address_to_nonce.is_empty() {
                    //     tracing::info!("   🔢 NONCE UPDATES: {} contract(s)", result.state_diff.address_to_nonce.len());
                    // }
                    // if !result.state_diff.storage_updates.is_empty() {
                    //     let total: usize = result.state_diff.storage_updates.values().map(|v| v.len()).sum();
                    //     tracing::info!("   💾 STORAGE UPDATES: {} update(s)", total);
                    // }
                    // if !result.state_diff.address_to_class_hash.is_empty() {
                    //     tracing::info!("   🏗️  CLASS HASH UPDATES: {} deployment(s)", result.state_diff.address_to_class_hash.len());
                    // }
                }

                Ok(result)
            })
            .collect::<Result<Vec<_>, _>>()
    }
}

/// Fallback to call-tree verification for non-Invoke transactions.
#[cfg(feature = "rust-verification")]
fn call_tree_verification_fallback<D: mc_db::MadaraStorageRead>(
    verification_adapter: &crate::BlockifierStateAdapter<D>,
    result: &ExecutionResult,
) -> crate::rust_exec_integration::CallTreeVerificationResult {
    use crate::rust_exec_integration::CallTreeVerificationResult;

    let mut verification_result =
        if let Some(execute_call_info) = &result.execution_info.execute_call_info {
            let block_timestamp = verification_adapter.block_number; // Approximate
            crate::rust_exec_integration::verify_call_tree_with_timestamp(
                verification_adapter,
                execute_call_info,
                result,
                None,
                block_timestamp,
            )
        } else {
            CallTreeVerificationResult {
                any_executed: false,
                all_passed: true,
                total_execution_duration: std::time::Duration::ZERO,
                call_timings: Vec::new(),
            }
        };

    // Also verify fee transfer call if present
    if let Some(fee_transfer_call_info) = &result.execution_info.fee_transfer_call_info {
        let block_timestamp = verification_adapter.block_number;
        let fee_result = crate::rust_exec_integration::verify_call_tree_with_timestamp(
            verification_adapter,
            fee_transfer_call_info,
            result,
            Some("Fee Transfer"),
            block_timestamp,
        );
        if fee_result.any_executed {
            verification_result.any_executed = true;
            verification_result.total_execution_duration += fee_result.total_execution_duration;
            verification_result.call_timings.extend(fee_result.call_timings);
        }
    }

    verification_result
}

/// Helper function to count events in a call tree recursively.
fn count_events_in_call_tree(call_info: &blockifier::execution::call_info::CallInfo) -> usize {
    let mut count = call_info.execution.events.len();
    for inner_call in &call_info.inner_calls {
        count += count_events_in_call_tree(inner_call);
    }
    count
}

/// Helper function to count L2-to-L1 messages in a call tree recursively.
fn count_messages_in_call_tree(call_info: &blockifier::execution::call_info::CallInfo) -> usize {
    let mut count = call_info.execution.l2_to_l1_messages.len();
    for inner_call in &call_info.inner_calls {
        count += count_messages_in_call_tree(inner_call);
    }
    count
}

pub trait TxInfo {
    fn contract_address(&self) -> ContractAddress;
    fn tx_hash(&self) -> TransactionHash;
    fn tx_nonce(&self) -> Option<Nonce>;
    fn tx_type(&self) -> TransactionType;
    fn fee_type(&self) -> FeeType;
    fn tip(&self) -> Option<Tip>;
    fn is_only_query(&self) -> bool;
    fn deployed_contract_address(&self) -> Option<ContractAddress>;
    fn declared_class_hash(&self) -> Option<ClassHash>;
    fn declared_contract_class(&self) -> Option<(ClassHash, ContractClass)>;
    fn l1_handler_tx_nonce(&self) -> Option<Nonce>;
}

impl TxInfo for Transaction {
    fn tx_hash(&self) -> TransactionHash {
        Self::tx_hash(self)
    }

    fn tx_nonce(&self) -> Option<Nonce> {
        if self.tx_type() == TransactionType::L1Handler {
            None
        } else {
            Some(Self::nonce(self))
        }
    }

    fn contract_address(&self) -> ContractAddress {
        Self::sender_address(self)
    }

    fn is_only_query(&self) -> bool {
        match self {
            Self::Account(tx) => tx.execution_flags.only_query,
            Self::L1Handler(_) => false,
        }
    }

    fn tx_type(&self) -> TransactionType {
        match self {
            Self::Account(tx) => tx.tx_type(),
            Self::L1Handler(_) => TransactionType::L1Handler,
        }
    }

    fn tip(&self) -> Option<Tip> {
        match self {
            // function tip() is only available for Account transactions with version 3 otherwise it panics.
            Self::Account(tx) if tx.version() >= TransactionVersion::THREE => Some(tx.tip()),
            _ => None,
        }
    }

    fn fee_type(&self) -> FeeType {
        match self {
            Self::Account(tx) => tx.fee_type(),
            Self::L1Handler(tx) => tx.fee_type(),
        }
    }

    fn declared_class_hash(&self) -> Option<ClassHash> {
        match self {
            Self::Account(tx) => tx.class_hash(),
            _ => None,
        }
    }

    fn declared_contract_class(&self) -> Option<(ClassHash, ContractClass)> {
        match self {
            Self::Account(AccountTransaction { tx: ApiAccountTransaction::Declare(tx), .. }) => {
                Some((tx.class_hash(), tx.class_info.contract_class.clone()))
            }
            _ => None,
        }
    }

    fn deployed_contract_address(&self) -> Option<ContractAddress> {
        match self {
            Self::Account(AccountTransaction { tx: ApiAccountTransaction::DeployAccount(tx), .. }) => {
                Some(tx.contract_address)
            }
            _ => None,
        }
    }

    fn l1_handler_tx_nonce(&self) -> Option<Nonce> {
        match self {
            Self::L1Handler(tx) => Some(tx.tx.nonce),
            _ => None,
        }
    }
}
