//! Execution context for tracking state changes during execution.
//!
//! The ExecutionContext tracks all storage reads, writes, events, and other
//! side effects during contract execution, then produces a final result.

use indexmap::IndexMap;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

use crate::state::{StateError, StateReader};
use crate::types::{
    CallExecutionResult, ContractAddress, Event, ExecutionResult, L2ToL1Message, Nonce, StateDiff, StorageKey,
};

/// Tracks all state changes during contract execution.
#[derive(Debug, Default)]
pub struct ExecutionContext {
    /// Storage reads: (contract, key) -> value (first read only)
    initial_reads: HashMap<(ContractAddress, StorageKey), Felt>,

    /// Storage writes: (contract, key) -> new_value
    storage_writes: HashMap<(ContractAddress, StorageKey), Felt>,

    /// Nonce updates: contract -> new_nonce
    nonce_updates: HashMap<ContractAddress, Nonce>,

    /// Events emitted (in order)
    events: Vec<Event>,

    /// L2 to L1 messages
    messages: Vec<L2ToL1Message>,

    /// Return data from execution
    retdata: Vec<Felt>,

    /// Whether execution failed
    failed: bool,

    /// Error message if failed
    error: Option<String>,

    /// Event counter for ordering
    event_counter: usize,

    /// Estimated gas consumed
    gas_consumed: u64,

    /// Block timestamp (for functions that need it)
    block_timestamp: u64,
}

impl ExecutionContext {
    /// Create a new execution context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new execution context with block timestamp.
    pub fn with_timestamp(block_timestamp: u64) -> Self {
        Self { block_timestamp, ..Self::default() }
    }

    /// Get the block timestamp.
    pub fn block_timestamp(&self) -> u64 {
        self.block_timestamp
    }

    /// Read a storage value.
    ///
    /// If we've already written to this key in this execution, returns the written value.
    /// Otherwise reads from the underlying state and caches the initial read.
    pub fn storage_read<S: StateReader>(
        &mut self,
        state: &S,
        contract: ContractAddress,
        key: StorageKey,
    ) -> Result<Felt, StateError> {
        // Check if we've already written to this key
        if let Some(value) = self.storage_writes.get(&(contract, key)) {
            tracing::info!(
                "📖 storage_read: key={:#x} → returning from storage_writes={:#x} (NOT tracking in initial_reads)",
                key.0, value
            );
            return Ok(*value);
        }

        // Check if we've already read this key
        if let Some(value) = self.initial_reads.get(&(contract, key)) {
            tracing::info!(
                "📖 storage_read: key={:#x} → returning cached read={:#x}",
                key.0, value
            );
            return Ok(*value);
        }

        // Read from underlying state
        let value = state.get_storage_at(contract, key)?;

        // Cache the initial read
        self.initial_reads.insert((contract, key), value);

        tracing::info!(
            "📖 storage_read: key={:#x} → reading from state={:#x}, ADDED to initial_reads",
            key.0, value
        );

        // Estimate gas for storage read
        self.gas_consumed += 100;

        Ok(value)
    }

    /// Write a storage value.
    pub fn storage_write(&mut self, contract: ContractAddress, key: StorageKey, value: Felt) {
        self.storage_writes.insert((contract, key), value);

        // Estimate gas for storage write
        self.gas_consumed += 200;
    }

    /// Emit an event.
    ///
    /// Events are ordered by the order they are emitted.
    pub fn emit_event(&mut self, keys: Vec<Felt>, data: Vec<Felt>) {
        self.events.push(Event { order: self.event_counter, keys, data });
        self.event_counter += 1;

        // Estimate gas for event
        self.gas_consumed +=
            50 + (self.events.last().unwrap().keys.len() + self.events.last().unwrap().data.len()) as u64 * 10;
    }

    /// Send a message to L1.
    pub fn send_message_to_l1(&mut self, to_address: Felt, payload: Vec<Felt>) {
        self.messages.push(L2ToL1Message { to_address, payload });

        // Estimate gas for L1 message
        self.gas_consumed += 500;
    }

    /// Set the return data.
    pub fn set_retdata(&mut self, retdata: Vec<Felt>) {
        self.retdata = retdata;
    }

    /// Mark execution as failed with an error message.
    pub fn fail(&mut self, error: String) {
        self.failed = true;
        self.error = Some(error);
    }

    /// Increment nonce for an account.
    pub fn increment_nonce<S: StateReader>(&mut self, state: &S, account: ContractAddress) -> Result<(), StateError> {
        let current_nonce = state.get_nonce_at(account)?;
        let new_nonce = current_nonce.increment();
        self.nonce_updates.insert(account, new_nonce);
        Ok(())
    }

    /// Get the current gas consumed estimate.
    pub fn gas_consumed(&self) -> u64 {
        self.gas_consumed
    }

    /// Check if execution has failed.
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Merge a state diff into this context.
    /// This is used when executing nested calls that return their own state changes.
    pub fn merge_state_diff(&mut self, state_diff: &StateDiff) {
        // Merge storage updates
        for (contract, updates) in &state_diff.storage_updates {
            for (key, value) in updates {
                self.storage_writes.insert((*contract, *key), *value);
            }
        }

        // Merge nonce updates
        for (address, nonce) in &state_diff.address_to_nonce {
            self.nonce_updates.insert(*address, *nonce);
        }
    }

    /// Merge events from a nested call.
    pub fn merge_events(&mut self, events: Vec<Event>) {
        for event in events {
            self.events.push(Event {
                order: self.event_counter,
                keys: event.keys,
                data: event.data,
            });
            self.event_counter += 1;
        }
    }

    /// Build the final state diff.
    ///
    /// Only includes storage values that actually changed OR non-zero initializations.
    /// Matches Blockifier: writes of 0 to unread slots are excluded.
    pub fn build_state_diff(&self) -> StateDiff {
        let mut storage_updates: IndexMap<ContractAddress, IndexMap<StorageKey, Felt>> = IndexMap::new();

        for ((contract, key), new_value) in &self.storage_writes {
            // Check if we read this slot before writing
            if let Some(old_value) = self.initial_reads.get(&(*contract, *key)) {
                // We read it first - only include if value changed
                if old_value != new_value {
                    tracing::info!(
                        "✅ Including in diff (changed): contract={:#x} key={:#x} old={:#x} new={:#x}",
                        contract.0, key.0, old_value, new_value
                    );
                    storage_updates.entry(*contract).or_default().insert(*key, *new_value);
                } else {
                    tracing::info!(
                        "⏭️  Skipping (unchanged): contract={:#x} key={:#x} val={:#x}",
                        contract.0, key.0, new_value
                    );
                }
            } else {
                // Never read before writing - only include if NON-ZERO
                // (Blockifier excludes writes of 0 to unread slots)
                if *new_value != Felt::ZERO {
                    tracing::info!(
                        "✅ Including in diff (unread non-zero): contract={:#x} key={:#x} val={:#x}",
                        contract.0, key.0, new_value
                    );
                    storage_updates.entry(*contract).or_default().insert(*key, *new_value);
                } else {
                    tracing::info!(
                        "⏭️  Skipping (unread zero): contract={:#x} key={:#x}",
                        contract.0, key.0
                    );
                }
            }
        }

        StateDiff {
            storage_updates,
            address_to_nonce: IndexMap::from_iter(self.nonce_updates.clone()),
            address_to_class_hash: IndexMap::new(),
            class_hash_to_compiled_class_hash: IndexMap::new(),
        }
    }

    /// Build the final execution result.
    pub fn build_result(&self) -> ExecutionResult {
        ExecutionResult {
            call_result: CallExecutionResult {
                retdata: self.retdata.clone(),
                events: self.events.clone(),
                l2_to_l1_messages: self.messages.clone(),
                failed: self.failed,
                gas_consumed: self.gas_consumed,
            },
            state_diff: self.build_state_diff(),
            revert_error: self.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_storage_read_caches_initial_value() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));
        state.set_storage(contract, key, Felt::from(42u64));

        let mut ctx = ExecutionContext::new();

        // First read should go to state
        let value = ctx.storage_read(&state, contract, key).unwrap();
        assert_eq!(value, Felt::from(42u64));

        // Second read should return cached value
        let value2 = ctx.storage_read(&state, contract, key).unwrap();
        assert_eq!(value2, Felt::from(42u64));
    }

    #[test]
    fn test_storage_write_then_read() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));

        let mut ctx = ExecutionContext::new();

        // Write a value
        ctx.storage_write(contract, key, Felt::from(99u64));

        // Read should return written value
        let value = ctx.storage_read(&state, contract, key).unwrap();
        assert_eq!(value, Felt::from(99u64));
    }

    #[test]
    fn test_state_diff_only_includes_changes() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));
        state.set_storage(contract, key, Felt::from(42u64));

        let mut ctx = ExecutionContext::new();

        // Read then write same value - should not appear in diff
        let _ = ctx.storage_read(&state, contract, key).unwrap();
        ctx.storage_write(contract, key, Felt::from(42u64));

        let diff = ctx.build_state_diff();
        assert!(diff.storage_updates.is_empty());
    }

    #[test]
    fn test_state_diff_excludes_unread_zero_writes() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));

        let mut ctx = ExecutionContext::new();

        // Write 0 without reading first - should NOT appear in diff (Blockifier behavior)
        ctx.storage_write(contract, key, Felt::from(0u64));

        let diff = ctx.build_state_diff();
        assert!(diff.storage_updates.is_empty());
    }

    #[test]
    fn test_state_diff_includes_unread_nonzero_writes() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));

        let mut ctx = ExecutionContext::new();

        // Write non-zero without reading first - SHOULD appear in diff
        ctx.storage_write(contract, key, Felt::from(42u64));

        let diff = ctx.build_state_diff();
        assert_eq!(diff.storage_updates.len(), 1);
        assert_eq!(*diff.storage_updates.get(&contract).unwrap().get(&key).unwrap(), Felt::from(42u64));
    }

    #[test]
    fn test_state_diff_includes_actual_changes() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));
        state.set_storage(contract, key, Felt::from(42u64));

        let mut ctx = ExecutionContext::new();

        // Read then write different value - should appear in diff
        let _ = ctx.storage_read(&state, contract, key).unwrap();
        ctx.storage_write(contract, key, Felt::from(99u64));

        let diff = ctx.build_state_diff();
        assert_eq!(diff.storage_updates.len(), 1);
        assert_eq!(*diff.storage_updates.get(&contract).unwrap().get(&key).unwrap(), Felt::from(99u64));
    }

    #[test]
    fn test_events_are_ordered() {
        let mut ctx = ExecutionContext::new();

        ctx.emit_event(vec![Felt::from(1u64)], vec![]);
        ctx.emit_event(vec![Felt::from(2u64)], vec![]);
        ctx.emit_event(vec![Felt::from(3u64)], vec![]);

        let result = ctx.build_result();
        assert_eq!(result.call_result.events.len(), 3);
        assert_eq!(result.call_result.events[0].order, 0);
        assert_eq!(result.call_result.events[1].order, 1);
        assert_eq!(result.call_result.events[2].order, 2);
    }
}
