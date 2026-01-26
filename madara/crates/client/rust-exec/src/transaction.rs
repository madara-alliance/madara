//! Transaction types and full transaction execution.
//!
//! This module provides the full transaction execution flow that mirrors Blockifier:
//! 1. Account __validate__
//! 2. Increment nonce
//! 3. Account __execute__ (which calls target contracts)
//! 4. Calculate fee
//! 5. Transfer fee to sequencer

use starknet_types_core::felt::Felt;

use crate::contracts::ExecutionError;
use crate::gas::{calculate_fee, BlockContext, FeeType, GasTracker, GasVector, ResourceBounds};
use crate::state::StateReader;
use crate::types::{CallExecutionResult, ContractAddress, ExecutionResult, Nonce, StateDiff};

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
    /// Create a new transaction executor.
    pub fn new(state: &'a S, block_context: &'a BlockContext) -> Self {
        Self { state, block_context, gas_tracker: GasTracker::new() }
    }

    /// Execute a full invoke transaction.
    ///
    /// This runs the complete flow:
    /// 1. __validate__ on account
    /// 2. Increment nonce
    /// 3. __execute__ on account (dispatches to target contracts)
    /// 4. Calculate fee from gas consumed
    /// 5. Transfer fee from sender to sequencer
    pub fn execute_invoke(
        &mut self,
        tx: &InvokeTransaction,
        account_class_hash: Felt,
    ) -> Result<TransactionExecutionResult, ExecutionError> {
        let mut combined_state_diff = StateDiff::default();

        // 1. Validate transaction
        let validate_result = self.execute_validate(tx, account_class_hash)?;

        // Merge validate state diff (if any storage changes)
        if let Some(ref _result) = validate_result {
            // Usually validate doesn't change state, but merge anyway
        }

        // 2. Increment nonce
        self.increment_nonce(tx.sender_address, &mut combined_state_diff)?;

        // 3. Execute transaction - charge gas for account's __execute__
        self.gas_tracker.charge_call_contract();
        for call in &tx.calls {
            self.gas_tracker.charge_calldata(call.calldata.len());
            self.gas_tracker.charge_call_contract();
        }

        // Execute all calls and collect state changes + events
        let mut all_events = Vec::new();
        for call in &tx.calls {
            // Execute each call and collect its state diff + events
            let call_result = self.execute_single_call(call, tx.sender_address)?;

            // Merge state diff
            combined_state_diff.merge(call_result.state_diff);

            // Collect events
            all_events.extend(call_result.call_result.events);
        }

        // Build execute call result with aggregated events
        let execute_result = CallExecutionResult {
            retdata: vec![],  // Transaction-level retdata not needed
            events: all_events,
            l2_to_l1_messages: vec![],
            failed: false,
            gas_consumed: 0,  // Tracked separately in gas_tracker
        };

        // 4. Calculate fee
        self.gas_tracker.calculate_da_gas(&combined_state_diff);
        let gas_consumed = self.gas_tracker.gas_vector();
        let actual_fee = calculate_fee(&gas_consumed, self.block_context, tx.fee_type);

        // 5. Transfer fee
        let fee_transfer_result = self.transfer_fee(tx, actual_fee, &mut combined_state_diff)?;

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
            retdata: vec![],
            events: vec![],
            l2_to_l1_messages: vec![],
            failed: false,
            gas_consumed: 5000,
        }))
    }

    /// Increment the account nonce.
    fn increment_nonce(&mut self, account: ContractAddress, state_diff: &mut StateDiff) -> Result<(), ExecutionError> {
        let current_nonce = self.state.get_nonce_at(account)?;
        let new_nonce = current_nonce.increment();
        state_diff.address_to_nonce.insert(account, new_nonce);
        Ok(())
    }

    /// Execute a single call to a contract.
    fn execute_single_call(
        &mut self,
        call: &Call,
        caller: ContractAddress,
    ) -> Result<ExecutionResult, ExecutionError> {
        // Get the class hash for the target contract
        let class_hash = self
            .state
            .get_class_hash_at(call.to)?
            .ok_or_else(|| ExecutionError::ExecutionFailed(format!("Contract not deployed: {:?}", call.to)))?;

        // Try to execute with our Rust implementation
        if let Some(result) = crate::contracts::ContractRegistry::execute(
            self.state,
            call.to,
            class_hash,
            call.selector,
            &call.calldata,
            caller,  // ← FIXED: Pass the real caller (account), not the contract
        ) {
            let exec_result = result?;

            // Charge gas for this call
            self.gas_tracker.charge_computation(exec_result.call_result.gas_consumed);

            return Ok(exec_result);
        }

        // Contract not supported - return empty result
        Ok(ExecutionResult {
            call_result: CallExecutionResult {
                retdata: vec![],
                events: vec![],
                l2_to_l1_messages: vec![],
                failed: false,
                gas_consumed: 0,
            },
            state_diff: StateDiff::default(),
            revert_error: None,
        })
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

    #[test]
    fn test_call_creation() {
        let call = Call {
            to: ContractAddress(Felt::from(1u64)),
            selector: Felt::from(2u64),
            calldata: vec![Felt::from(3u64)],
        };
        assert_eq!(call.calldata.len(), 1);
    }
}
