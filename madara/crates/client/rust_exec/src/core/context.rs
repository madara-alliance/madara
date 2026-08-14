//! Execution context for tracking state changes during execution.
//!
//! The ExecutionContext tracks all storage reads, writes, events, and other
//! side effects during contract execution, then produces a final result.

use indexmap::IndexMap;
use starknet_types_core::felt::Felt;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::core::state::{StateError, StateReader};
use crate::core::types::{
    CallExecutionResult, ContractAddress, Event, ExecutionResult, L2ToL1Message, Nonce, StateDiff, StorageKey,
};
use crate::telemetry::hash_agg::{self, CtxReadSource};
use crate::telemetry::storage_agg::{self, CtxReadLayer};

/// Tracks all state changes during contract execution.
#[derive(Debug, Default)]
pub struct ExecutionContext {
    /// Storage reads: (contract, key) -> value (first read only)
    initial_reads: HashMap<(ContractAddress, StorageKey), Felt>,

    /// Per-tx storage read cache (after first backend read)
    storage_read_cache: HashMap<(ContractAddress, StorageKey), Felt>,

    /// Storage writes: (contract, key) -> new_value
    storage_writes: HashMap<(ContractAddress, StorageKey), Felt>,

    /// Zero writes already validated by a nested state diff.
    preserved_nested_zero_writes: HashSet<(ContractAddress, StorageKey)>,

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

    /// Storage read counters (for debugging/perf analysis)
    storage_reads_total: u64,
    storage_reads_from_writes: u64,
    storage_reads_backend: u64,
    storage_writes_total: u64,
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
        self.storage_reads_total = self.storage_reads_total.saturating_add(1);
        let storage_agg_enabled = storage_agg::enabled();
        let read_cache_enabled = crate::config::ctx_cache_enabled();
        let start = if storage_agg_enabled { Some(Instant::now()) } else { None };
        // Check write cache first, then per-tx read cache (single timing bucket).
        enum CacheHit {
            Write(Felt),
            Read(Felt),
        }
        let cache_hit = if let Some(value) = self.storage_writes.get(&(contract, key)).copied() {
            Some(CacheHit::Write(value))
        } else if read_cache_enabled {
            self.storage_read_cache.get(&(contract, key)).copied().map(CacheHit::Read)
        } else {
            None
        };
        if let Some(hit) = cache_hit {
            match hit {
                CacheHit::Write(value) => {
                    self.storage_reads_from_writes = self.storage_reads_from_writes.saturating_add(1);
                    hash_agg::record_ctx_read(CtxReadSource::WriteCache);
                    if let Some(start) = start {
                        let elapsed_us = start.elapsed().as_micros() as u64;
                        storage_agg::record_ctx_read(CtxReadLayer::WriteCache, contract, key, elapsed_us);
                    }
                    return Ok(value);
                }
                CacheHit::Read(value) => {
                    hash_agg::record_ctx_read(CtxReadSource::ReadCache);
                    if let Some(start) = start {
                        let elapsed_us = start.elapsed().as_micros() as u64;
                        storage_agg::record_ctx_read(CtxReadLayer::ReadCache, contract, key, elapsed_us);
                    }
                    return Ok(value);
                }
            }
        }

        // Read from underlying state
        self.storage_reads_backend = self.storage_reads_backend.saturating_add(1);
        hash_agg::record_ctx_read(CtxReadSource::Backend);
        let pre_elapsed_us = start.map(|start| start.elapsed().as_micros() as u64).unwrap_or(0);
        let value = state.get_storage_at(contract, key)?;

        // Track the initial read (first read only) for diff correctness.
        if storage_agg_enabled {
            let post_start = Instant::now();
            self.initial_reads.entry((contract, key)).or_insert(value);
            if read_cache_enabled {
                self.storage_read_cache.insert((contract, key), value);
            }
            let post_elapsed_us = post_start.elapsed().as_micros() as u64;
            let layer_us = pre_elapsed_us.saturating_add(post_elapsed_us);
            storage_agg::record_ctx_read(CtxReadLayer::Backend, contract, key, layer_us);
        } else {
            self.initial_reads.entry((contract, key)).or_insert(value);
            if read_cache_enabled {
                self.storage_read_cache.insert((contract, key), value);
            }
        }

        // Estimate gas for storage read
        self.gas_consumed += 100;

        Ok(value)
    }

    /// Write a storage value.
    pub fn storage_write(&mut self, contract: ContractAddress, key: StorageKey, value: Felt) {
        let storage_agg_enabled = storage_agg::enabled();
        let start = if storage_agg_enabled { Some(Instant::now()) } else { None };
        self.storage_writes_total = self.storage_writes_total.saturating_add(1);
        self.preserved_nested_zero_writes.remove(&(contract, key));
        self.storage_writes.insert((contract, key), value);
        hash_agg::record_ctx_write();
        if let Some(start) = start {
            let elapsed_us = start.elapsed().as_micros() as u64;
            storage_agg::record_ctx_write(contract, key, elapsed_us);
        }

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
                if *value == Felt::ZERO {
                    self.preserved_nested_zero_writes.insert((*contract, *key));
                } else {
                    self.preserved_nested_zero_writes.remove(&(*contract, *key));
                }
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
            self.events.push(Event { order: self.event_counter, keys: event.keys, data: event.data });
            self.event_counter += 1;
        }
    }

    /// Merge L2->L1 messages from a nested call.
    pub fn merge_messages(&mut self, messages: Vec<L2ToL1Message>) {
        for msg in messages {
            self.messages.push(msg);
        }
    }

    /// Build the final state diff.
    ///
    /// Only includes storage values that actually changed OR non-zero initializations.
    /// Matches Blockifier: writes of 0 to unread slots are excluded.
    pub fn build_state_diff(&self) -> StateDiff {
        let mut storage_updates: IndexMap<ContractAddress, IndexMap<StorageKey, Felt>> = IndexMap::new();

        for ((contract, key), new_value) in &self.storage_writes {
            if self.preserved_nested_zero_writes.contains(&(*contract, *key)) {
                storage_updates.entry(*contract).or_default().insert(*key, *new_value);
                continue;
            }

            // Check if we read this slot before writing
            if let Some(old_value) = self.initial_reads.get(&(*contract, *key)) {
                // We read it first - only include if value changed
                if old_value != new_value {
                    storage_updates.entry(*contract).or_default().insert(*key, *new_value);
                }
            } else {
                // Never read before writing - only include if NON-ZERO
                // (Blockifier excludes writes of 0 to unread slots)
                if *new_value != Felt::ZERO {
                    storage_updates.entry(*contract).or_default().insert(*key, *new_value);
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

    /// Snapshot storage read/write counters.
    pub fn storage_read_stats(&self) -> StorageReadStats {
        StorageReadStats {
            total: self.storage_reads_total,
            from_writes: self.storage_reads_from_writes,
            backend: self.storage_reads_backend,
            writes: self.storage_writes_total,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StorageReadStats {
    pub total: u64,
    pub from_writes: u64,
    pub backend: u64,
    pub writes: u64,
}

impl StorageReadStats {
    pub fn diff(self, before: StorageReadStats) -> StorageReadStats {
        StorageReadStats {
            total: self.total.saturating_sub(before.total),
            from_writes: self.from_writes.saturating_sub(before.from_writes),
            backend: self.backend.saturating_sub(before.backend),
            writes: self.writes.saturating_sub(before.writes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::mock::MockStateReader;

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
        let _state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));

        let mut ctx = ExecutionContext::new();

        // Write 0 without reading first - should NOT appear in diff (Blockifier behavior)
        ctx.storage_write(contract, key, Felt::from(0u64));

        let diff = ctx.build_state_diff();
        assert!(diff.storage_updates.is_empty());
    }

    #[test]
    fn test_state_diff_preserves_nested_zero_update() {
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));
        let nested_diff = StateDiff {
            storage_updates: IndexMap::from_iter([(contract, IndexMap::from_iter([(key, Felt::ZERO)]))]),
            ..StateDiff::default()
        };

        let mut ctx = ExecutionContext::new();
        ctx.merge_state_diff(&nested_diff);

        let diff = ctx.build_state_diff();
        assert_eq!(diff.storage_updates[&contract][&key], Felt::ZERO);
    }

    #[test]
    fn test_direct_write_replaces_nested_zero_semantics() {
        let contract = ContractAddress(Felt::from(1u64));
        let key = StorageKey(Felt::from(100u64));
        let nested_diff = StateDiff {
            storage_updates: IndexMap::from_iter([(contract, IndexMap::from_iter([(key, Felt::ZERO)]))]),
            ..StateDiff::default()
        };

        let mut ctx = ExecutionContext::new();
        ctx.merge_state_diff(&nested_diff);
        ctx.storage_write(contract, key, Felt::ZERO);

        assert!(ctx.build_state_diff().storage_updates.is_empty());
    }

    #[test]
    fn test_state_diff_includes_unread_nonzero_writes() {
        let _state = MockStateReader::new();
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
