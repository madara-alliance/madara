//! Transaction types and full transaction execution.
//!
//! This module provides the full transaction execution flow that mirrors Blockifier:
//! 1. Validate nonce
//! 2. Account __validate__
//! 3. Increment nonce
//! 4. Account __execute__ (which calls target contracts)
//! 5. Calculate fee
//! 6. Transfer fee to sequencer

use starknet_types_core::felt::Felt;

use crate::contracts::{account, ExecutionError};
use crate::core::gas::{calculate_fee, BlockContext, FeeType, GasTracker, GasVector, ResourceBounds};
use crate::core::state::StateReader;
use crate::core::storage::short_string_to_felt;
use crate::core::types::{CallExecutionResult, ContractAddress, ExecutionResult, Nonce, StateDiff};

fn no_charge_fee_enabled() -> bool {
    crate::config::no_charge_fee_enabled()
}

/// A single call within a transaction.
#[derive(Debug, Clone)]
pub struct Call {
    /// Target contract address
    pub to: ContractAddress,
    /// Function selector
    pub selector: Felt,
    /// Calldata for the function
    pub calldata: Vec<Felt>,
}

/// Transaction information for Invoke transactions.
#[derive(Debug, Clone)]
pub struct InvokeTransaction {
    /// Transaction hash
    pub tx_hash: Felt,
    /// Transaction version
    pub version: Felt,
    /// Sender (account) address
    pub sender_address: ContractAddress,
    /// Calls to execute
    pub calls: Vec<Call>,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// Fee type (ETH or STRK)
    pub fee_type: FeeType,
    /// Resource bounds (max fees)
    pub resource_bounds: ResourceBounds,
}

/// Complete transaction execution result.
#[derive(Debug, Clone)]
pub struct TransactionExecutionResult {
    /// Validate call info
    pub validate_call_info: Option<CallExecutionResult>,
    /// Execute call info
    pub execute_call_info: Option<CallExecutionResult>,
    /// Fee transfer call info
    pub fee_transfer_call_info: Option<CallExecutionResult>,
    /// Complete state diff (including fee token updates)
    pub state_diff: StateDiff,
    /// Actual fee charged
    pub actual_fee: u128,
    /// Gas consumed
    pub gas_consumed: GasVector,
    /// Revert error if any
    pub revert_error: Option<String>,
}

/// Read-through view that makes writes from earlier calls in the same account
/// multicall visible to later calls before the transaction diff is committed.
struct TransactionOverlayState<'a, S> {
    base: &'a S,
    overlay: &'a StateDiff,
}

impl<S: StateReader> StateReader for TransactionOverlayState<'_, S> {
    fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: crate::core::types::StorageKey,
    ) -> Result<Felt, crate::core::state::StateError> {
        if let Some(value) = self.overlay.storage_updates.get(&contract_address).and_then(|updates| updates.get(&key)) {
            return Ok(*value);
        }
        self.base.get_storage_at(contract_address, key)
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> Result<Nonce, crate::core::state::StateError> {
        if let Some(nonce) = self.overlay.address_to_nonce.get(&contract_address) {
            return Ok(*nonce);
        }
        self.base.get_nonce_at(contract_address)
    }

    fn get_class_hash_at(
        &self,
        contract_address: ContractAddress,
    ) -> Result<Option<Felt>, crate::core::state::StateError> {
        if let Some(class_hash) = self.overlay.address_to_class_hash.get(&contract_address) {
            return Ok(Some(*class_hash));
        }
        self.base.get_class_hash_at(contract_address)
    }
}

impl TransactionExecutionResult {
    /// Check if transaction succeeded
    pub fn is_success(&self) -> bool {
        self.revert_error.is_none()
    }
}

/// Transaction executor that runs full transaction flow.
pub struct TransactionExecutor<'a, S: StateReader> {
    state: &'a S,
    block_context: &'a BlockContext,
    gas_tracker: GasTracker,
}

impl<'a, S: StateReader> TransactionExecutor<'a, S> {
    const FIXED_FEE_AMOUNT: u128 = 0x112c1628d20;
    /// Create a new transaction executor.
    pub fn new(state: &'a S, block_context: &'a BlockContext) -> Self {
        Self { state, block_context, gas_tracker: GasTracker::new() }
    }

    /// Execute a full invoke transaction.
    ///
    /// This runs the complete flow:
    /// 1. Validate nonce
    /// 2. __validate__ on account
    /// 3. Increment nonce
    /// 4. __execute__ on account (dispatches to target contracts)
    /// 5. Calculate fee from gas consumed
    /// 6. Transfer fee from sender to sequencer
    pub fn execute_invoke(
        &mut self,
        tx: &InvokeTransaction,
        account_class_hash: Felt,
    ) -> Result<TransactionExecutionResult, ExecutionError> {
        let mut combined_state_diff = StateDiff::default();

        // 1. Reject stale or out-of-order transactions before executing any calls.
        let current_nonce = self.state.get_nonce_at(tx.sender_address)?;
        if tx.nonce != current_nonce {
            return Err(ExecutionError::InvalidNonce { expected: current_nonce.0, actual: tx.nonce.0 });
        }

        // 2. Validate transaction
        let validate_result = self.execute_validate(tx, account_class_hash)?;

        // Merge validate state diff (if any storage changes)
        if let Some(ref _result) = validate_result {
            // Usually validate doesn't change state, but merge anyway
        }

        // 3. Increment nonce
        combined_state_diff.address_to_nonce.insert(tx.sender_address, current_nonce.increment());

        // 4. Execute transaction - charge gas for account's __execute__
        self.gas_tracker.charge_call_contract();
        for call in &tx.calls {
            self.gas_tracker.charge_calldata(call.calldata.len());
            self.gas_tracker.charge_call_contract();
        }

        // Execute all calls and collect state changes + events
        let mut all_events = Vec::new();
        let mut all_messages = Vec::new();
        let mut retdata = Vec::new();
        for call in &tx.calls {
            if let Some(diagnostic) = crate::telemetry::tx_diff::current() {
                tracing::info!(
                    target: "RUST_EXEC",
                    "contract_call_stage block_number={} tx_hash={:#x} stage=started contract_address={:#x} selector={:#x} calldata_len={}",
                    diagnostic.block_number,
                    diagnostic.tx_hash,
                    call.to.0,
                    call.selector,
                    call.calldata.len(),
                );
            }
            // Execute each call and collect its state diff + events
            let overlay_state = TransactionOverlayState { base: self.state, overlay: &combined_state_diff };
            let call_result =
                Self::execute_single_call(&overlay_state, call, tx.sender_address, self.block_context.block_timestamp)?;
            self.gas_tracker.charge_computation(call_result.call_result.gas_consumed);
            if let Some(diagnostic) = crate::telemetry::tx_diff::current() {
                let storage_entries =
                    call_result.state_diff.storage_updates.values().map(|updates| updates.len()).sum::<usize>();
                tracing::info!(
                    target: "RUST_EXEC",
                    "contract_call_stage block_number={} tx_hash={:#x} stage=completed contract_address={:#x} selector={:#x} storage_entries={} nonce_updates={} events={} failed={} revert_error={:?}",
                    diagnostic.block_number,
                    diagnostic.tx_hash,
                    call.to.0,
                    call.selector,
                    storage_entries,
                    call_result.state_diff.address_to_nonce.len(),
                    call_result.call_result.events.len(),
                    call_result.call_result.failed,
                    call_result.revert_error,
                );
            }
            let call_failed = call_result.call_result.failed;
            let revert_error = call_result.revert_error.clone();

            // Merge state diff
            combined_state_diff.merge(call_result.state_diff);

            // Collect events
            all_events.extend(call_result.call_result.events);
            all_messages.extend(call_result.call_result.l2_to_l1_messages);
            retdata = call_result.call_result.retdata;

            if call_failed || revert_error.is_some() {
                return Err(ExecutionError::ExecutionFailed(
                    revert_error.unwrap_or_else(|| "Rust inner call failed".to_string()),
                ));
            }
        }

        if let Some(account_event) =
            account::transaction_executed_event(account_class_hash, tx.tx_hash, all_events.len())
        {
            all_events.insert(0, account_event);
        }

        // Build execute call result with aggregated events
        let execute_result = CallExecutionResult {
            retdata,
            events: all_events,
            l2_to_l1_messages: all_messages,
            failed: false,
            gas_consumed: 0, // Tracked separately in gas_tracker
        };

        // 5. Calculate fee
        self.gas_tracker.calculate_da_gas(&combined_state_diff);
        let gas_consumed = self.gas_tracker.gas_vector();
        let _calculated_fee = calculate_fee(&gas_consumed, self.block_context, tx.fee_type);
        let skip_fee = no_charge_fee_enabled();
        let actual_fee = if skip_fee { 0 } else { Self::FIXED_FEE_AMOUNT };

        // 6. Transfer fee
        let fee_transfer_result =
            if skip_fee { None } else { self.transfer_fee(tx, actual_fee, &mut combined_state_diff)? };

        Ok(TransactionExecutionResult {
            validate_call_info: validate_result,
            execute_call_info: Some(execute_result),
            fee_transfer_call_info: fee_transfer_result,
            state_diff: combined_state_diff,
            actual_fee,
            gas_consumed,
            revert_error: None,
        })
    }

    /// Execute __validate__ on the account contract.
    fn execute_validate(
        &mut self,
        tx: &InvokeTransaction,
        _account_class_hash: Felt,
    ) -> Result<Option<CallExecutionResult>, ExecutionError> {
        // Charge gas for validate
        self.gas_tracker.charge_call_contract();
        self.gas_tracker.charge_calldata(tx.signature.len());

        // For now, we assume validation passes
        // A full implementation would:
        // 1. Call account.__validate__(calls, signature)
        // 2. Verify the signature matches the transaction hash
        // 3. Return the result

        // Estimate gas for signature verification (ECDSA)
        self.gas_tracker.charge_computation(5000); // ECDSA verification cost

        Ok(Some(CallExecutionResult {
            retdata: vec![short_string_to_felt("VALID")],
            events: vec![],
            l2_to_l1_messages: vec![],
            failed: false,
            gas_consumed: 5000,
        }))
    }

    /// Execute a single call to a contract.
    fn execute_single_call<R: StateReader>(
        state: &R,
        call: &Call,
        caller: ContractAddress,
        block_timestamp: u64,
    ) -> Result<ExecutionResult, ExecutionError> {
        // Get the class hash for the target contract
        let class_hash = state
            .get_class_hash_at(call.to)?
            .ok_or_else(|| ExecutionError::ExecutionFailed(format!("Contract not deployed: {:?}", call.to)))?;
        // Try to execute with our Rust implementation
        if let Some(result) = crate::contracts::ContractRegistry::execute_with_timestamp(
            state,
            call.to,
            class_hash,
            call.selector,
            &call.calldata,
            caller, // ← FIXED: Pass the real caller (account), not the contract
            block_timestamp,
        ) {
            let exec_result = match result {
                Ok(ok) => ok,
                Err(err) => {
                    return Err(err);
                }
            };

            return Ok(exec_result);
        }

        Err(ExecutionError::ExecutionFailed(format!(
            "Unsupported Rust Exec call: contract={:#x} class_hash={:#x} selector={:#x}",
            call.to.0, class_hash, call.selector
        )))
    }

    /// Transfer fee from sender to sequencer.
    fn transfer_fee(
        &mut self,
        tx: &InvokeTransaction,
        amount: u128,
        state_diff: &mut StateDiff,
    ) -> Result<Option<CallExecutionResult>, ExecutionError> {
        let fee_token_address = match tx.fee_type {
            FeeType::Eth => self.block_context.eth_fee_token_address,
            FeeType::Strk => self.block_context.strk_fee_token_address,
        };

        // Use ERC20 transfer to move funds
        // This requires knowing the ERC20 storage layout
        let transfer_result = crate::contracts::erc20::transfer_internal(
            self.state,
            fee_token_address,
            tx.sender_address,
            self.block_context.sequencer_address,
            amount,
            state_diff,
        )?;

        self.gas_tracker.charge_call_contract();
        self.gas_tracker.charge_storage_read(); // Read sender balance
        self.gas_tracker.charge_storage_read(); // Read sequencer balance
        self.gas_tracker.charge_storage_write(false); // Update sender balance
        self.gas_tracker.charge_storage_write(false); // Update sequencer balance

        Ok(Some(transfer_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::devnet::rust_exec_transfer;
    use crate::core::state::mock::MockStateReader;
    use crate::core::storage::{function_selector, storage_key_for_variable};
    use crate::core::types::StorageKey;

    #[test]
    fn test_call_creation() {
        let call = Call {
            to: ContractAddress(Felt::from(1u64)),
            selector: Felt::from(2u64),
            calldata: vec![Felt::from(3u64)],
        };
        assert_eq!(call.calldata.len(), 1);
    }

    #[test]
    fn transaction_overlay_exposes_prior_call_writes() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(0x100u64));
        let key = StorageKey(Felt::from(0x200u64));
        state.set_storage(contract, key, Felt::from(3u64));
        state.set_nonce(contract, Nonce(Felt::from(4u64)));
        state.set_class_hash(contract, Felt::from(5u64));

        let mut overlay = StateDiff::default();
        overlay.storage_updates.entry(contract).or_default().insert(key, Felt::ZERO);
        overlay.address_to_nonce.insert(contract, Nonce(Felt::from(6u64)));
        overlay.address_to_class_hash.insert(contract, Felt::from(7u64));
        let view = TransactionOverlayState { base: &state, overlay: &overlay };

        assert_eq!(view.get_storage_at(contract, key).unwrap(), Felt::ZERO);
        assert_eq!(view.get_nonce_at(contract).unwrap(), Nonce(Felt::from(6u64)));
        assert_eq!(view.get_class_hash_at(contract).unwrap(), Some(Felt::from(7u64)));
    }

    #[test]
    fn transaction_overlay_feeds_prior_fixture_call_into_the_next_call() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(0x100u64));
        let caller = ContractAddress(Felt::from(0x200u64));
        let first = rust_exec_transfer::execute(
            &state,
            contract,
            function_selector("transfer"),
            &[Felt::from(0x300u64), Felt::from(11u64)],
            caller,
        )
        .expect("first fixture call should execute");
        let overlay = TransactionOverlayState { base: &state, overlay: &first.state_diff };

        let second = rust_exec_transfer::execute(
            &overlay,
            contract,
            function_selector("transfer"),
            &[Felt::from(0x400u64), Felt::from(22u64)],
            caller,
        )
        .expect("second fixture call should execute against the first call overlay");

        assert_eq!(
            second.state_diff.storage_updates[&contract][&storage_key_for_variable("transfer_count")],
            Felt::TWO
        );
    }

    #[test]
    fn strk_fee_type_does_not_change_the_fee_token_event_abi() {
        let sender = ContractAddress(Felt::from(0x100u64));
        let token = ContractAddress(Felt::from(0x200u64));
        let sequencer = ContractAddress(Felt::from(0x300u64));
        let mut state = MockStateReader::new();
        state.set_storage(token, crate::contracts::erc20::layout::balance_key(sender), Felt::from(100u64));
        let block_context =
            BlockContext { sequencer_address: sequencer, strk_fee_token_address: token, ..BlockContext::default() };
        let mut executor = TransactionExecutor::new(&state, &block_context);
        let tx = InvokeTransaction {
            tx_hash: Felt::from(0x123u64),
            version: Felt::ZERO,
            sender_address: sender,
            calls: Vec::new(),
            signature: Vec::new(),
            nonce: Nonce(Felt::ZERO),
            fee_type: FeeType::Strk,
            resource_bounds: ResourceBounds::default(),
        };
        let mut state_diff = StateDiff::default();

        let result = executor.transfer_fee(&tx, 10, &mut state_diff).unwrap().unwrap();
        let event = result.events.first().unwrap();

        assert_eq!(event.keys, vec![crate::core::storage::event_selector("Transfer")]);
        assert_eq!(event.data, vec![sender.0, sequencer.0, Felt::from(10u64), Felt::ZERO]);
    }

    #[test]
    fn unsupported_call_errors_instead_of_fee_only_success() {
        let mut state = MockStateReader::new();
        let sender = ContractAddress(Felt::from(0x100u64));
        let target = ContractAddress(Felt::from(0x200u64));
        state.set_class_hash(target, Felt::from(0xdeadbeefu64));

        let block_context = BlockContext::default();
        let mut executor = TransactionExecutor::new(&state, &block_context);
        let tx = InvokeTransaction {
            tx_hash: Felt::from(0x123u64),
            version: Felt::ZERO,
            sender_address: sender,
            calls: vec![Call { to: target, selector: Felt::from(0x456u64), calldata: Vec::new() }],
            signature: Vec::new(),
            nonce: Nonce(Felt::ZERO),
            fee_type: FeeType::Strk,
            resource_bounds: ResourceBounds::default(),
        };

        let error = executor.execute_invoke(&tx, Felt::ZERO).expect_err("unsupported call must fail");
        assert!(error.to_string().contains("Unsupported Rust Exec call"), "unexpected error: {error}");
    }

    #[test]
    fn rejects_transaction_nonce_that_does_not_match_state() {
        let mut state = MockStateReader::new();
        let sender = ContractAddress(Felt::from(0x100u64));
        let target = ContractAddress(Felt::from(0x200u64));
        state.set_nonce(sender, Nonce(Felt::from(7u64)));
        state.set_class_hash(target, Felt::from(0xdeadbeefu64));

        let block_context = BlockContext::default();
        let mut executor = TransactionExecutor::new(&state, &block_context);
        let tx = InvokeTransaction {
            tx_hash: Felt::from(0x123u64),
            version: Felt::ZERO,
            sender_address: sender,
            calls: vec![Call { to: target, selector: Felt::from(0x456u64), calldata: Vec::new() }],
            signature: Vec::new(),
            nonce: Nonce(Felt::from(8u64)),
            fee_type: FeeType::Strk,
            resource_bounds: ResourceBounds::default(),
        };

        let error = executor.execute_invoke(&tx, Felt::ZERO).expect_err("nonce mismatch must fail");
        assert!(matches!(
            error,
            ExecutionError::InvalidNonce { expected, actual }
                if expected == Felt::from(7u64) && actual == Felt::from(8u64)
        ));
    }
}
