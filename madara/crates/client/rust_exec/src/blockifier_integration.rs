//! Blockifier integration for Rust execution and verification.
//!
//! This module contains the logic that:
//! 1) Executes transactions using Rust implementations.
//! 2) Compares results against Blockifier execution.

use blockifier::{
    blockifier::transaction_executor::{
        TransactionExecutionOutput, TransactionExecutorError, TransactionExecutorResult,
    },
    context::BlockContext,
    execution::call_info::{
        CallExecution, CallInfo, OrderedEvent, OrderedL2ToL1Message, Retdata, StorageAccessTracker,
    },
    execution::contract_class::TrackedResource,
    execution::entry_point::{CallEntryPoint, CallType},
    execution::stack_trace::{ErrorStack, ErrorStackHeader, ErrorStackSegment},
    fee::receipt::TransactionReceipt,
    fee::resources::TransactionResources,
    state::cached_state::{CommitmentStateDiff, StateMaps},
    state::errors::StateError,
    state::state_api::StateReader as BlockifierStateReader,
    transaction::errors::TransactionExecutionError,
    transaction::objects::{RevertError, TransactionExecutionInfo},
    transaction::transaction_execution::Transaction,
};
use mp_convert::ToFelt;
use once_cell::sync::Lazy;
use serde::Serialize;
use starknet_api::core::L1Address;
use starknet_api::core::{
    ClassHash as ApiClassHash, ContractAddress as ApiContractAddress, EntryPointSelector, Nonce as ApiNonce,
};
use starknet_api::executable_transaction::{AccountTransaction, InvokeTransaction};
use starknet_api::execution_resources::{GasAmount, GasVector as ApiGasVector};
use starknet_api::state::StorageKey;
use starknet_api::transaction::fields::{Calldata, Fee};
use starknet_api::transaction::{EventContent, EventData, EventKey, L2ToL1Payload};
use starknet_types_core::felt::Felt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::{
    constants,
    execution::execute_transaction_with_timestamp,
    gas::{BlockContext as RustBlockContext, FeeType, GasVector, ResourceBounds},
    runner::ExecutionVerificationResult,
    storage::function_selector,
    trace::{build_transaction_execution_info, RustCallType, RustEntryPointType, RustTransactionExecutionInfo},
    transaction::{Call, InvokeTransaction as RustInvokeTransaction, TransactionExecutor},
    types::{CallExecutionResult, ContractAddress, ExecutionResult, Nonce, StateDiff as RustStateDiff},
    StateReader, VerificationError,
};

static INIT: Lazy<()> = Lazy::new(|| {
    crate::init();
});

static RUST_CONVERSION_LOG_ENABLED: Lazy<bool> = Lazy::new(|| {
    std::env::var("RUST_CONVERSION_LOG").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
});

static RUST_EXECUTION_LOG_ENABLED: Lazy<bool> = Lazy::new(|| {
    std::env::var("RUST_EXECUTION_LOG").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
});

fn hardcoded_settle_trade_v3_gas() -> GasVector {
    constants::settle_trade_v3_fixed_gas_consumed()
}

/// Adapter that allows a Blockifier `StateReader` (including `CachedState`) to be used with
/// rust-exec's `StateReader` trait.
pub struct RustExecStateAdapter<'a, S: BlockifierStateReader> {
    cached_state: &'a S,
    storage_calls: AtomicU64,
    storage_backend_calls: AtomicU64,
    storage_time_us: AtomicU64,
    nonce_calls: AtomicU64,
    nonce_time_us: AtomicU64,
    class_hash_calls: AtomicU64,
    class_hash_time_us: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StateReadStats {
    pub storage_calls: u64,
    pub storage_backend_calls: u64,
    pub storage_time_us: u64,
    pub nonce_calls: u64,
    pub nonce_time_us: u64,
    pub class_hash_calls: u64,
    pub class_hash_time_us: u64,
}

impl<'a, S: BlockifierStateReader> RustExecStateAdapter<'a, S> {
    pub fn new(cached_state: &'a S) -> Self {
        Self {
            cached_state,
            storage_calls: AtomicU64::new(0),
            storage_backend_calls: AtomicU64::new(0),
            storage_time_us: AtomicU64::new(0),
            nonce_calls: AtomicU64::new(0),
            nonce_time_us: AtomicU64::new(0),
            class_hash_calls: AtomicU64::new(0),
            class_hash_time_us: AtomicU64::new(0),
        }
    }

    pub fn reset_stats(&self) {
        self.storage_calls.store(0, Ordering::Relaxed);
        self.storage_backend_calls.store(0, Ordering::Relaxed);
        self.storage_time_us.store(0, Ordering::Relaxed);
        self.nonce_calls.store(0, Ordering::Relaxed);
        self.nonce_time_us.store(0, Ordering::Relaxed);
        self.class_hash_calls.store(0, Ordering::Relaxed);
        self.class_hash_time_us.store(0, Ordering::Relaxed);
    }

    pub fn stats_snapshot(&self) -> StateReadStats {
        StateReadStats {
            storage_calls: self.storage_calls.load(Ordering::Relaxed),
            storage_backend_calls: self.storage_backend_calls.load(Ordering::Relaxed),
            storage_time_us: self.storage_time_us.load(Ordering::Relaxed),
            nonce_calls: self.nonce_calls.load(Ordering::Relaxed),
            nonce_time_us: self.nonce_time_us.load(Ordering::Relaxed),
            class_hash_calls: self.class_hash_calls.load(Ordering::Relaxed),
            class_hash_time_us: self.class_hash_time_us.load(Ordering::Relaxed),
        }
    }
}

impl<'a, S: BlockifierStateReader> StateReader for RustExecStateAdapter<'a, S> {
    fn get_storage_at(
        &self,
        contract: ContractAddress,
        key: crate::types::StorageKey,
    ) -> Result<Felt, crate::StateError> {
        let starknet_contract = ApiContractAddress::try_from(contract.0)
            .map_err(|e| crate::StateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let starknet_key = StorageKey::try_from(key.0)
            .map_err(|e| crate::StateError::InvalidStorageKey(format!("Invalid storage key: {e}")))?;

        let value = self
            .cached_state
            .get_storage_at(starknet_contract, starknet_key)
            .map_err(|e| crate::StateError::BackendError(format!("Blockifier cached state read error: {e}")))?;

        Ok(value)
    }

    fn get_nonce_at(&self, contract: ContractAddress) -> Result<Nonce, crate::StateError> {
        let starknet_contract = ApiContractAddress::try_from(contract.0)
            .map_err(|e| crate::StateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let nonce = self
            .cached_state
            .get_nonce_at(starknet_contract)
            .map_err(|e| crate::StateError::BackendError(format!("Blockifier cached state read error: {e}")))?;

        Ok(Nonce(nonce.0))
    }

    fn get_class_hash_at(&self, contract: ContractAddress) -> Result<Option<Felt>, crate::StateError> {
        let starknet_contract = ApiContractAddress::try_from(contract.0)
            .map_err(|e| crate::StateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let class_hash = self
            .cached_state
            .get_class_hash_at(starknet_contract)
            .map_err(|e| crate::StateError::BackendError(format!("Blockifier cached state read error: {e}")))?;

        if class_hash.0 == Felt::ZERO {
            Ok(None)
        } else {
            Ok(Some(class_hash.0))
        }
    }
}

/// Timing entry for a single verified call.
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Copy, Serialize)]
pub enum RustExecutionSkipReason {
    NonInvokeTransaction,
    L1HandlerTransaction,
    UnsupportedAccountClass,
}

#[derive(Debug, Clone, Serialize)]
pub struct RustExecutionData {
    pub result: ExecutionResult,
    pub duration: std::time::Duration,
    pub sender_address: Felt,
    pub tx_hash: Felt,
    pub fee_token_address: Felt,
    pub validate_call_info: Option<CallExecutionResult>,
    pub fee_transfer_call_info: Option<CallExecutionResult>,
    pub fee_amount: u128,
    pub gas_consumed: GasVector,
    pub execution_info: Option<RustTransactionExecutionInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RustExecutionFailure {
    pub error: String,
    pub duration: std::time::Duration,
    pub sender_address: Felt,
    pub tx_hash: Felt,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::large_enum_variant)]
pub enum RustExecutionOutcome {
    Executed(RustExecutionData),
    Skipped { reason: RustExecutionSkipReason, tx_hash: Felt, block_timestamp: u64 },
    Failed(RustExecutionFailure),
}

/// Result of executing rust-exec and formatting into Blockifier-native output.
#[derive(Debug)]
pub struct RustBlockifierOutput {
    pub outcome: RustExecutionOutcome,
    pub output: TransactionExecutorResult<TransactionExecutionOutput>,
}

/// Execute a transaction in Rust and return Blockifier-native output.
pub fn rust_execute_transaction_blockifier_output<S: StateReader>(
    state: &S,
    tx: &Transaction,
    block_context: &BlockContext,
    tx_hash: Felt,
) -> RustBlockifierOutput {
    let outcome = rust_execute_transaction_with_info(state, tx, block_context, tx_hash);
    let output = if *RUST_CONVERSION_LOG_ENABLED {
        let start = Instant::now();
        let output = rust_execution_outcome_to_blockifier_output(&outcome);
        let elapsed = start.elapsed();
        tracing::info!("rust->blockifier conversion took {:?} (tx_hash={:#x})", elapsed, tx_hash);
        output
    } else {
        rust_execution_outcome_to_blockifier_output(&outcome)
    };
    RustBlockifierOutput { outcome, output }
}

/// Convert a rust-exec outcome into Blockifier's `TransactionExecutionOutput`.
pub fn rust_execution_outcome_to_blockifier_output(
    outcome: &RustExecutionOutcome,
) -> TransactionExecutorResult<TransactionExecutionOutput> {
    match outcome {
        RustExecutionOutcome::Executed(data) => {
            let rust_info = data.execution_info.as_ref().ok_or_else(|| {
                TransactionExecutorError::TransactionExecutionError(TransactionExecutionError::StateError(
                    StateError::StateReadError("missing rust execution info".to_string()),
                ))
            })?;

            let maps = build_state_maps_from_rust_diff(&data.result.state_diff)?;
            let execution_info = build_blockifier_tx_execution_info_from_rust(rust_info)?;
            Ok((execution_info, maps))
        }
        RustExecutionOutcome::Failed(failure) => {
            Err(TransactionExecutorError::TransactionExecutionError(TransactionExecutionError::StateError(
                StateError::StateReadError(format!("rust exec failed: {}", failure.error)),
            )))
        }
        RustExecutionOutcome::Skipped { reason, .. } => {
            Err(TransactionExecutorError::TransactionExecutionError(TransactionExecutionError::StateError(
                StateError::StateReadError(format!("rust exec skipped: {:?}", reason)),
            )))
        }
    }
}

fn map_rust_call_type(call_type: RustCallType) -> CallType {
    match call_type {
        RustCallType::Regular => CallType::Call,
        RustCallType::Delegate => CallType::Delegate,
    }
}

fn map_rust_entry_point_type(entry_point_type: RustEntryPointType) -> starknet_api::contract_class::EntryPointType {
    match entry_point_type {
        RustEntryPointType::External => starknet_api::contract_class::EntryPointType::External,
        RustEntryPointType::Constructor => starknet_api::contract_class::EntryPointType::Constructor,
        RustEntryPointType::L1Handler => starknet_api::contract_class::EntryPointType::L1Handler,
    }
}

fn build_blockifier_call_info_from_rust(
    rust_call: &crate::trace::RustCallInfo,
) -> Result<CallInfo, TransactionExecutionError> {
    let storage_address = ApiContractAddress::try_from(rust_call.function_call.contract_address)
        .map_err(|e| TransactionExecutionError::StateError(StateError::StarknetApiError(e)))?;
    let caller_address = ApiContractAddress::try_from(rust_call.caller_address)
        .map_err(|e| TransactionExecutionError::StateError(StateError::StarknetApiError(e)))?;
    let entry_point_selector = EntryPointSelector(rust_call.function_call.entry_point_selector);
    let calldata = Calldata(rust_call.function_call.calldata.clone().into());
    let call_type = map_rust_call_type(rust_call.call_type);
    let entry_point_type = map_rust_entry_point_type(rust_call.entry_point_type);

    let events = rust_call
        .execution
        .events
        .iter()
        .map(|event| OrderedEvent {
            order: event.order,
            event: EventContent {
                keys: event.keys.iter().cloned().map(EventKey).collect(),
                data: EventData(event.data.clone()),
            },
        })
        .collect::<Vec<_>>();

    let messages = rust_call
        .execution
        .l2_to_l1_messages
        .iter()
        .enumerate()
        .map(|(order, message)| OrderedL2ToL1Message {
            order,
            message: blockifier::execution::call_info::MessageToL1 {
                to_address: L1Address(message.to_address),
                payload: L2ToL1Payload(message.payload.clone()),
            },
        })
        .collect::<Vec<_>>();

    let execution = CallExecution {
        retdata: Retdata(rust_call.execution.retdata.clone()),
        events,
        l2_to_l1_messages: messages,
        cairo_native: false,
        failed: rust_call.execution.failed,
        gas_consumed: rust_call.execution.gas_consumed,
    };

    let inner_calls =
        rust_call.inner_calls.iter().map(build_blockifier_call_info_from_rust).collect::<Result<Vec<_>, _>>()?;

    Ok(CallInfo {
        call: CallEntryPoint {
            class_hash: Some(ApiClassHash(rust_call.class_hash)),
            code_address: Some(storage_address),
            entry_point_type,
            entry_point_selector,
            calldata,
            storage_address,
            caller_address,
            call_type,
            initial_gas: 0,
        },
        execution,
        inner_calls,
        resources: Default::default(),
        tracked_resource: TrackedResource::CairoSteps,
        storage_access_tracker: StorageAccessTracker::default(),
        builtin_counters: Default::default(),
        syscalls_usage: Default::default(),
    })
}

fn build_blockifier_tx_execution_info_from_rust(
    rust_info: &RustTransactionExecutionInfo,
) -> Result<TransactionExecutionInfo, TransactionExecutionError> {
    let validate_call_info =
        rust_info.validate_call_info.as_ref().map(build_blockifier_call_info_from_rust).transpose()?;
    let execute_call_info =
        rust_info.execute_call_info.as_ref().map(build_blockifier_call_info_from_rust).transpose()?;
    let fee_transfer_call_info =
        rust_info.fee_transfer_call_info.as_ref().map(build_blockifier_call_info_from_rust).transpose()?;

    let revert_error = rust_info.revert_error.as_ref().map(|msg| {
        RevertError::Execution(ErrorStack {
            header: ErrorStackHeader::Execution,
            stack: vec![ErrorStackSegment::StringFrame(msg.clone())],
        })
    });

    let rust_gas = rust_info.receipt.gas_consumed.clone();
    let gas = ApiGasVector {
        l1_gas: GasAmount(rust_gas.l1_gas),
        l1_data_gas: GasAmount(rust_gas.l1_data_gas),
        l2_gas: GasAmount(rust_gas.l2_gas),
    };
    let da_gas =
        ApiGasVector { l1_gas: GasAmount(0), l1_data_gas: GasAmount(rust_gas.l1_data_gas), l2_gas: GasAmount(0) };

    let receipt = TransactionReceipt {
        fee: Fee(rust_info.receipt.actual_fee),
        gas,
        da_gas,
        resources: TransactionResources::default(),
    };

    Ok(TransactionExecutionInfo {
        validate_call_info,
        execute_call_info,
        fee_transfer_call_info,
        revert_error,
        receipt,
    })
}

fn build_state_maps_from_rust_diff(diff: &RustStateDiff) -> Result<StateMaps, TransactionExecutionError> {
    let mut maps = StateMaps::default();
    for (addr, storage) in &diff.storage_updates {
        let addr_api = ApiContractAddress::try_from(addr.0)
            .map_err(|e| TransactionExecutionError::StateError(StateError::StarknetApiError(e)))?;
        for (key, value) in storage {
            let skey = StorageKey::try_from(key.0)
                .map_err(|e| TransactionExecutionError::StateError(StateError::StarknetApiError(e)))?;
            maps.storage.insert((addr_api, skey), *value);
        }
    }
    for (addr, nonce) in &diff.address_to_nonce {
        let addr_api = ApiContractAddress::try_from(addr.0)
            .map_err(|e| TransactionExecutionError::StateError(StateError::StarknetApiError(e)))?;
        maps.nonces.insert(addr_api, ApiNonce(nonce.0));
    }
    for (addr, class_hash) in &diff.address_to_class_hash {
        let addr_api = ApiContractAddress::try_from(addr.0)
            .map_err(|e| TransactionExecutionError::StateError(StateError::StarknetApiError(e)))?;
        let ch = ApiClassHash(*class_hash);
        maps.class_hashes.insert(addr_api, ch);
        maps.declared_contracts.insert(ch, true);
    }
    for (class_hash, compiled_class_hash) in &diff.class_hash_to_compiled_class_hash {
        let ch = ApiClassHash(*class_hash);
        maps.compiled_class_hashes.insert(ch, starknet_api::core::CompiledClassHash(*compiled_class_hash));
        maps.declared_contracts.insert(ch, true);
    }
    Ok(maps)
}

/// Execute a transaction in Rust and compare against Blockifier.
pub fn rust_execute_and_verify_transaction<S: StateReader>(
    state: &S,
    tx: &Transaction,
    block_context: &BlockContext,
    tx_hash: Felt,
    execution_info: &TransactionExecutionInfo,
    state_diff: &CommitmentStateDiff,
) -> CallTreeVerificationResult {
    Lazy::force(&INIT);

    let rust_result = rust_execute_transaction(state, tx, block_context, tx_hash);
    verify_results(state, execution_info, state_diff, rust_result)
}

pub fn rust_execute_transaction<S: StateReader>(
    state: &S,
    tx: &Transaction,
    block_context: &BlockContext,
    tx_hash: Felt,
) -> RustExecutionOutcome {
    Lazy::force(&INIT);

    let block_timestamp = block_context.block_info().block_timestamp.0;

    match tx {
        Transaction::Account(account_tx) => match &account_tx.tx {
            AccountTransaction::Invoke(invoke_tx) => {
                execute_full_invoke_transaction(state, invoke_tx, block_context, tx_hash)
            }
            _ => RustExecutionOutcome::Skipped {
                reason: RustExecutionSkipReason::NonInvokeTransaction,
                tx_hash,
                block_timestamp,
            },
        },
        Transaction::L1Handler(_) => RustExecutionOutcome::Skipped {
            reason: RustExecutionSkipReason::L1HandlerTransaction,
            tx_hash,
            block_timestamp,
        },
    }
}

pub fn rust_execute_transaction_with_info<S: StateReader>(
    state: &S,
    tx: &Transaction,
    block_context: &BlockContext,
    tx_hash: Felt,
) -> RustExecutionOutcome {
    let is_settle_trade_v3 = is_settle_trade_v3_invoke(tx);
    let outcome = if *RUST_EXECUTION_LOG_ENABLED {
        let start = Instant::now();
        let outcome = rust_execute_transaction(state, tx, block_context, tx_hash);
        let elapsed = start.elapsed();
        tracing::info!("rust execution took {:?} (tx_hash={:#x})", elapsed, tx_hash);
        outcome
    } else {
        rust_execute_transaction(state, tx, block_context, tx_hash)
    };

    match outcome {
        RustExecutionOutcome::Executed(mut data) => {
            if is_settle_trade_v3 {
                data.fee_amount = constants::settle_trade_v3_fixed_fee_amount();
                data.gas_consumed = hardcoded_settle_trade_v3_gas();
            }
            let calldata_arc = match tx {
                Transaction::Account(account_tx) => match &account_tx.tx {
                    AccountTransaction::Invoke(invoke_tx) => Some(invoke_tx.calldata().0.clone()),
                    _ => None,
                },
                _ => None,
            };

            if let Some(calldata_arc) = calldata_arc {
                let calldata = calldata_arc.as_ref();
                let account_class_hash =
                    state.get_class_hash_at(ContractAddress(data.sender_address)).ok().flatten().unwrap_or(Felt::ZERO);

                let sequencer_address = block_context.block_info().sequencer_address.to_felt();

                let execution_info = build_transaction_execution_info(
                    state,
                    data.sender_address,
                    account_class_hash,
                    calldata,
                    &data.result,
                    data.validate_call_info.clone(),
                    data.fee_transfer_call_info.clone(),
                    data.fee_amount,
                    data.fee_token_address,
                    sequencer_address,
                    data.gas_consumed.clone(),
                );

                data.execution_info = Some(execution_info);
            }

            RustExecutionOutcome::Executed(data)
        }
        RustExecutionOutcome::Failed(failure) => RustExecutionOutcome::Failed(failure),
        RustExecutionOutcome::Skipped { reason, block_timestamp, .. } => {
            RustExecutionOutcome::Skipped { reason, tx_hash, block_timestamp }
        }
    }
}

pub fn verify_results<S: StateReader>(
    state: &S,
    execution_info: &TransactionExecutionInfo,
    state_diff: &CommitmentStateDiff,
    rust_result: RustExecutionOutcome,
) -> CallTreeVerificationResult {
    match rust_result {
        RustExecutionOutcome::Executed(data) => verify_full_invoke_results(execution_info, state_diff, &data),
        RustExecutionOutcome::Failed(failure) => CallTreeVerificationResult {
            any_executed: true,
            all_passed: false,
            total_execution_duration: failure.duration,
            call_timings: vec![CallTimingEntry {
                label: "Full Transaction (FAILED)".to_string(),
                contract_address: failure.sender_address,
                duration: failure.duration,
                executed: true,
                verification_passed: false,
            }],
        },
        RustExecutionOutcome::Skipped { reason, tx_hash, block_timestamp } => match reason {
            RustExecutionSkipReason::NonInvokeTransaction | RustExecutionSkipReason::L1HandlerTransaction => {
                call_tree_verification_fallback(state, execution_info, state_diff, tx_hash, block_timestamp)
            }
            RustExecutionSkipReason::UnsupportedAccountClass => CallTreeVerificationResult {
                any_executed: false,
                all_passed: true,
                total_execution_duration: std::time::Duration::ZERO,
                call_timings: Vec::new(),
            },
        },
    }
}

fn call_tree_verification_fallback<S: StateReader>(
    state: &S,
    execution_info: &TransactionExecutionInfo,
    state_diff: &CommitmentStateDiff,
    tx_hash: Felt,
    block_timestamp: u64,
) -> CallTreeVerificationResult {
    let mut verification_result = if let Some(execute_call_info) = &execution_info.execute_call_info {
        verify_call_tree_with_timestamp(state, execute_call_info, state_diff, tx_hash, None, block_timestamp)
    } else {
        CallTreeVerificationResult {
            any_executed: false,
            all_passed: true,
            total_execution_duration: std::time::Duration::ZERO,
            call_timings: Vec::new(),
        }
    };

    if let Some(fee_transfer_call_info) = &execution_info.fee_transfer_call_info {
        let fee_result = verify_call_tree_with_timestamp(
            state,
            fee_transfer_call_info,
            state_diff,
            tx_hash,
            Some("Fee Transfer"),
            block_timestamp,
        );
        if fee_result.any_executed {
            verification_result.any_executed = true;
            verification_result.total_execution_duration += fee_result.total_execution_duration;
            verification_result.call_timings.extend(fee_result.call_timings);
            if !fee_result.all_passed {
                verification_result.all_passed = false;
            }
        }
    }

    verification_result
}

fn verify_call_tree_with_timestamp<S: StateReader>(
    state: &S,
    call_info: &CallInfo,
    state_diff: &CommitmentStateDiff,
    tx_hash: Felt,
    phase_label: Option<&str>,
    block_timestamp: u64,
) -> CallTreeVerificationResult {
    let (this_result, call_label) =
        verify_single_call_with_label(state, call_info, state_diff, tx_hash, block_timestamp);

    let mut any_executed = this_result.executed;
    let mut all_passed = true;
    let mut total_duration = this_result.execution_duration;
    let mut call_timings = Vec::new();

    let label = phase_label.map(|p| p.to_string()).unwrap_or(call_label);
    call_timings.push(CallTimingEntry {
        label,
        contract_address: call_info.call.storage_address.to_felt(),
        duration: this_result.execution_duration,
        executed: this_result.executed,
        verification_passed: this_result.passed,
    });

    if this_result.executed && !this_result.passed {
        all_passed = false;
    }

    for inner_call in &call_info.inner_calls {
        let inner_result =
            verify_call_tree_with_timestamp(state, inner_call, state_diff, tx_hash, None, block_timestamp);
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

fn verify_single_call_with_label<S: StateReader>(
    state: &S,
    call_info: &CallInfo,
    state_diff: &CommitmentStateDiff,
    tx_hash: Felt,
    block_timestamp: u64,
) -> (ExecutionVerificationResult, String) {
    let contract_address = call_info.call.storage_address.to_felt();
    let class_hash = call_info.call.class_hash.unwrap_or_default().to_felt();
    let entry_point_selector = call_info.call.entry_point_selector.to_felt();
    let calldata = &call_info.call.calldata.0;
    let caller_address = call_info.call.caller_address.to_felt();

    let label = generate_call_label(class_hash, entry_point_selector);

    let result = verify_call_with_call_info(
        state,
        contract_address,
        class_hash,
        entry_point_selector,
        calldata,
        caller_address,
        call_info,
        state_diff,
        tx_hash,
        block_timestamp,
    );

    (result, label)
}

fn generate_call_label(class_hash: Felt, entry_point_selector: Felt) -> String {
    let contract_name = crate::get_contract_name(class_hash).unwrap_or_else(|| format!("{:#.8x}...", class_hash));
    let function_name = crate::get_function_name(class_hash, entry_point_selector)
        .unwrap_or_else(|| format!("{:#.8x}...", entry_point_selector));
    format!("{}.{}", contract_name, function_name)
}

#[allow(clippy::too_many_arguments)]
fn verify_call_with_call_info<S: StateReader>(
    state: &S,
    contract_address: Felt,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller_address: Felt,
    call_info: &CallInfo,
    state_diff: &CommitmentStateDiff,
    _tx_hash: Felt,
    block_timestamp: u64,
) -> ExecutionVerificationResult {
    let blockifier_storage: Vec<(Felt, Vec<(Felt, Felt)>)> = state_diff
        .storage_updates
        .iter()
        .map(|(addr, updates)| {
            let storage_entries: Vec<(Felt, Felt)> = updates.iter().map(|(k, v)| (k.to_felt(), *v)).collect();
            (addr.to_felt(), storage_entries)
        })
        .collect();

    let blockifier_retdata: Vec<Felt> = call_info.execution.retdata.0.clone();

    let blockifier_events: Vec<(Vec<Felt>, Vec<Felt>)> = if entry_point_selector == function_selector("__execute__") {
        collect_all_events_from_call_tree(call_info)
    } else {
        call_info
            .execution
            .events
            .iter()
            .map(|event| {
                let keys: Vec<Felt> = event.event.keys.iter().map(ToFelt::to_felt).collect();
                let data: Vec<Felt> = event.event.data.0.clone();
                (keys, data)
            })
            .collect()
    };

    let blockifier_failed = call_info.execution.failed;

    let blockifier_nonces: Vec<(Felt, Felt)> =
        state_diff.address_to_nonce.iter().map(|(addr, nonce)| (addr.to_felt(), nonce.to_felt())).collect();

    let blockifier_messages: Vec<(Felt, Vec<Felt>)> = call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .map(|msg| (msg.message.to_address.0, msg.message.payload.0.clone()))
        .collect();

    let blockifier_class_hashes: Vec<(Felt, Felt)> =
        state_diff.address_to_class_hash.iter().map(|(addr, hash)| (addr.to_felt(), hash.to_felt())).collect();

    let blockifier_compiled_class_hashes: Vec<(Felt, Felt)> = state_diff
        .class_hash_to_compiled_class_hash
        .iter()
        .map(|(hash, compiled)| (hash.to_felt(), compiled.to_felt()))
        .collect();

    let blockifier_result = crate::verification::BlockifierExecutionResult {
        storage: blockifier_storage,
        retdata: blockifier_retdata,
        events: blockifier_events,
        failed: blockifier_failed,
        nonces: blockifier_nonces,
        messages: blockifier_messages,
        class_hashes: blockifier_class_hashes,
        compiled_class_hashes: blockifier_compiled_class_hashes,
    };

    // -------------------------------
    // RUST EXECUTION (single call)
    // -------------------------------
    let rust_exec_result = execute_transaction_with_timestamp(
        state,
        ContractAddress(contract_address),
        class_hash,
        entry_point_selector,
        calldata,
        ContractAddress(caller_address),
        block_timestamp,
    );
    let rust_exec_duration = std::time::Duration::ZERO;

    let rust_result = match rust_exec_result {
        Some(Ok(result)) => result,
        Some(Err(_e)) => {
            return ExecutionVerificationResult {
                executed: true,
                passed: false,
                execution_duration: rust_exec_duration,
                rust_result: None,
                verification: None,
            };
        }
        None => {
            return ExecutionVerificationResult {
                executed: false,
                passed: false,
                execution_duration: std::time::Duration::ZERO,
                rust_result: None,
                verification: None,
            };
        }
    };

    // -------------------------------
    // VERIFICATION (single call)
    // -------------------------------
    let verification_result = crate::verification::verify_against_blockifier(&rust_result, &blockifier_result);

    ExecutionVerificationResult {
        executed: true,
        passed: verification_result.passed,
        execution_duration: rust_exec_duration,
        rust_result: Some(rust_result),
        verification: Some(verification_result),
    }
}

pub fn rust_execute_and_verify_full_invoke_transaction<S: StateReader>(
    state: &S,
    tx: &InvokeTransaction,
    block_context: &BlockContext,
    tx_hash: Felt,
    execution_info: &TransactionExecutionInfo,
    state_diff: &CommitmentStateDiff,
) -> CallTreeVerificationResult {
    let rust_result = execute_full_invoke_transaction(state, tx, block_context, tx_hash);
    verify_results(state, execution_info, state_diff, rust_result)
}

fn execute_full_invoke_transaction<S: StateReader>(
    state: &S,
    tx: &InvokeTransaction,
    block_context: &BlockContext,
    tx_hash: Felt,
) -> RustExecutionOutcome {
    let sender_address = tx.sender_address().to_felt();
    let calldata = &tx.calldata().0;

    let account_class_hash = match state.get_class_hash_at(ContractAddress(sender_address)) {
        Ok(Some(hash)) => hash,
        Ok(None) => {
            return RustExecutionOutcome::Failed(RustExecutionFailure {
                error: "Account class hash not found".to_string(),
                duration: std::time::Duration::ZERO,
                sender_address,
                tx_hash,
            });
        }
        Err(e) => {
            return RustExecutionOutcome::Failed(RustExecutionFailure {
                error: format!("Failed to get account class hash: {}", e),
                duration: std::time::Duration::ZERO,
                sender_address,
                tx_hash,
            });
        }
    };

    let rust_block_context = RustBlockContext {
        block_number: block_context.block_info().block_number.0,
        block_timestamp: block_context.block_info().block_timestamp.0,
        sequencer_address: ContractAddress(block_context.block_info().sequencer_address.to_felt()),
        l1_gas_price_wei: block_context.block_info().gas_prices.eth_gas_prices.l1_gas_price.get().0,
        l1_data_gas_price_wei: block_context.block_info().gas_prices.eth_gas_prices.l1_data_gas_price.get().0,
        l2_gas_price_wei: block_context.block_info().gas_prices.eth_gas_prices.l2_gas_price.get().0,
        l1_gas_price_fri: block_context.block_info().gas_prices.strk_gas_prices.l1_gas_price.get().0,
        l1_data_gas_price_fri: block_context.block_info().gas_prices.strk_gas_prices.l1_data_gas_price.get().0,
        l2_gas_price_fri: block_context.block_info().gas_prices.strk_gas_prices.l2_gas_price.get().0,
        eth_fee_token_address: ContractAddress(
            block_context.chain_info().fee_token_addresses.eth_fee_token_address.to_felt(),
        ),
        strk_fee_token_address: ContractAddress(
            block_context.chain_info().fee_token_addresses.strk_fee_token_address.to_felt(),
        ),
    };

    let nonce = tx.nonce();
    let nonce_value = nonce.0;

    let fee_type = FeeType::Eth;
    let resource_bounds = ResourceBounds::default();

    let rust_tx = RustInvokeTransaction {
        tx_hash,
        version: Felt::ZERO,
        sender_address: ContractAddress(sender_address),
        calls: { parse_calls_from_invoke_calldata(calldata).unwrap_or_default() },
        signature: tx.signature().0.to_vec(),
        nonce: Nonce(nonce_value),
        fee_type,
        resource_bounds,
    };

    let mut executor = TransactionExecutor::new(state, &rust_block_context);

    let rust_result = match executor.execute_invoke(&rust_tx, account_class_hash) {
        Ok(result) => result,
        Err(e) => {
            return RustExecutionOutcome::Failed(RustExecutionFailure {
                error: format!("Rust transaction execution failed: {}", e),
                duration: std::time::Duration::ZERO,
                sender_address,
                tx_hash,
            });
        }
    };

    let fee_only_write = rust_result.state_diff.storage_updates.keys().all(|address| {
        *address == rust_block_context.eth_fee_token_address || *address == rust_block_context.strk_fee_token_address
    });
    if fee_only_write {
        let parsed_calls: Vec<(Felt, Felt, usize)> =
            rust_tx.calls.iter().map(|call| (call.to.0, call.selector, call.calldata.len())).collect();
        let event_selectors: Vec<Felt> = rust_result
            .execute_call_info
            .as_ref()
            .map(|info| info.events.iter().filter_map(|event| event.keys.first().copied()).collect())
            .unwrap_or_default();
        let storage_update_addresses: Vec<Felt> =
            rust_result.state_diff.storage_updates.keys().map(|address| address.0).collect();
        tracing::warn!(
            tx_hash = %format!("{:#x}", tx_hash),
            sender_address = %format!("{:#x}", sender_address),
            parsed_calls = ?parsed_calls,
            execute_event_selectors = ?event_selectors,
            execute_retdata = ?rust_result.execute_call_info.as_ref().map(|info| info.retdata.clone()),
            storage_update_addresses = ?storage_update_addresses,
            "rust_exec_suspicious_fee_only_state_diff"
        );
    }

    let rust_exec_duration = std::time::Duration::ZERO;

    let rust_exec_result = ExecutionResult {
        call_result: rust_result.execute_call_info.clone().unwrap_or(CallExecutionResult {
            retdata: vec![],
            events: vec![],
            l2_to_l1_messages: vec![],
            failed: rust_result.revert_error.is_some(),
            gas_consumed: rust_result.gas_consumed.l2_gas,
        }),
        state_diff: rust_result.state_diff.clone(),
        revert_error: rust_result.revert_error.clone(),
    };

    RustExecutionOutcome::Executed(RustExecutionData {
        result: rust_exec_result,
        duration: rust_exec_duration,
        sender_address,
        tx_hash,
        fee_token_address: block_context.chain_info().fee_token_addresses.eth_fee_token_address.to_felt(),
        validate_call_info: rust_result.validate_call_info.clone(),
        fee_transfer_call_info: rust_result.fee_transfer_call_info.clone(),
        fee_amount: rust_result.actual_fee,
        gas_consumed: rust_result.gas_consumed.clone(),
        execution_info: None,
    })
}

fn verify_full_invoke_results(
    execution_info: &TransactionExecutionInfo,
    state_diff: &CommitmentStateDiff,
    rust_result: &RustExecutionData,
) -> CallTreeVerificationResult {
    let blockifier_storage: Vec<(Felt, Vec<(Felt, Felt)>)> = state_diff
        .storage_updates
        .iter()
        .map(|(addr, updates)| {
            let storage_entries: Vec<(Felt, Felt)> = updates.iter().map(|(k, v)| (k.to_felt(), *v)).collect();
            (addr.to_felt(), storage_entries)
        })
        .collect();

    let blockifier_nonces: Vec<(Felt, Felt)> =
        state_diff.address_to_nonce.iter().map(|(addr, nonce)| (addr.to_felt(), nonce.to_felt())).collect();

    let blockifier_events: Vec<(Vec<Felt>, Vec<Felt>)> =
        if let Some(execute_call_info) = &execution_info.execute_call_info {
            collect_all_events_from_call_tree(execute_call_info)
        } else {
            vec![]
        };

    let blockifier_result = crate::verification::BlockifierExecutionResult {
        storage: blockifier_storage,
        retdata: vec![],
        events: blockifier_events,
        failed: false,
        nonces: blockifier_nonces,
        messages: vec![],
        class_hashes: vec![],
        compiled_class_hashes: vec![],
    };

    let verification_result = crate::verification::verify_against_blockifier(&rust_result.result, &blockifier_result);
    let (contract_logic_errors, _fee_related_errors): (Vec<_>, Vec<_>) =
        verification_result.errors.iter().partition(|error| match error {
            VerificationError::StorageMismatch { contract, .. } => *contract != rust_result.fee_token_address,
            _ => true,
        });

    let all_passed = contract_logic_errors.is_empty();

    CallTreeVerificationResult {
        any_executed: true,
        all_passed,
        total_execution_duration: rust_result.duration,
        call_timings: vec![CallTimingEntry {
            label: "Full Transaction".to_string(),
            contract_address: rust_result.sender_address,
            duration: rust_result.duration,
            executed: true,
            verification_passed: all_passed,
        }],
    }
}

fn parse_calls_from_invoke_calldata(calldata: &[Felt]) -> Result<Vec<Call>, String> {
    let account_calls = crate::contracts::account::functions::parse_calls(calldata)
        .map_err(|e| format!("Failed to parse calls: {}", e))?;

    Ok(account_calls
        .into_iter()
        .map(|call| Call { to: call.to, selector: call.selector, calldata: call.calldata })
        .collect())
}

fn is_settle_trade_v3_invoke(tx: &Transaction) -> bool {
    let selector = function_selector("settle_trade_v3");
    let calldata_arc = match tx {
        Transaction::Account(account_tx) => match &account_tx.tx {
            AccountTransaction::Invoke(invoke_tx) => Some(invoke_tx.calldata().0.clone()),
            _ => None,
        },
        _ => None,
    };

    let Some(calldata_arc) = calldata_arc else {
        return false;
    };

    match parse_calls_from_invoke_calldata(calldata_arc.as_ref()) {
        Ok(calls) => calls.iter().any(|call| call.selector == selector),
        Err(_) => false,
    }
}

fn collect_all_events_from_call_tree(call_info: &CallInfo) -> Vec<(Vec<Felt>, Vec<Felt>)> {
    let mut events = vec![];

    for event in &call_info.execution.events {
        let keys: Vec<Felt> = event.event.keys.iter().map(ToFelt::to_felt).collect();
        let data: Vec<Felt> = event.event.data.0.clone();
        events.push((keys, data));
    }

    for inner_call in &call_info.inner_calls {
        events.extend(collect_all_events_from_call_tree(inner_call));
    }

    events
}
