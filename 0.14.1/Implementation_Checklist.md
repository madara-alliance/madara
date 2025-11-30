# Implementation Checklist for v0.14.1 Upgrade

This checklist provides a step-by-step guide for implementing all v0.14.1 changes in Madara.

## Prerequisites

- [ ] Review all documentation in `0.14.1/` directory
- [ ] Understand SNIP-34 requirements
- [ ] Review RPC 0.10.0 specification
- [ ] Set up test environment
- [ ] Create feature branch: `feature/v0.14.1-upgrade`

## Phase 1: SNIP-34 CASM Hash Migration

### 1.1 BLAKE Hash Implementation

- [ ] Research BLAKE hash implementation (check `casm_classes_v2` crate updates)
- [ ] Implement BLAKE hash calculation function
  - [ ] Create `madara/crates/primitives/class/src/blake_casm_hash.rs`
  - [ ] Implement `compute_blake_casm_hash()` function
  - [ ] Add unit tests for BLAKE hash calculation
- [ ] Update CASM compilation to use BLAKE
  - [ ] Modify `madara/crates/primitives/class/src/compile.rs`
  - [ ] Add version check for SNIP-34 activation
  - [ ] Integrate BLAKE hash calculation
- [ ] **REMINDER**: Update cairo0 OS code
  - [ ] Change default class-hash calculation back to BLAKE
  - [ ] Revert Poseidon change made for 0.14.0 compatibility
  - [ ] Verify cairo0 OS code works with SNIP-34

### 1.2 Migration Logic

- [ ] Add SNIP-34 activation block configuration
  - [ ] Update `madara/crates/primitives/chain_config/src/lib.rs`
  - [ ] Add `snip_34_activation_block` field
- [ ] Create migration database schema
  - [ ] Add `MigratedClassHash` struct
  - [ ] Update database schema
- [ ] Implement on-demand hash computation
  - [ ] Create infrastructure for on-demand computation
  - [ ] Implement `get_compiled_class_hash_v2()` with on-demand computation
  - [ ] Add caching mechanism for computed hashes
  - [ ] **CRITICAL**: Update class trie incrementally as classes are accessed
    - [ ] Recompute leaf hashes when class is accessed
    - [ ] Update class trie with new leaf hash values
    - [ ] Verify class commitment matches
  - [ ] Add tests for on-demand computation logic

### 1.3 Class Trie Updates

- [ ] Update class trie incrementally as classes are accessed
  - [ ] Update trie when `get_compiled_class_hash_v2()` computes a new hash
  - [ ] Recompute leaf hashes using new BLAKE `compiled_class_hash` values
  - [ ] Update class trie with new leaf hash values
  - [ ] Verify class commitment matches
- [ ] Note: Leaf hash calculation method (Poseidon) doesn't change
  - [ ] The input (`compiled_class_hash`) changes from Poseidon to BLAKE
  - [ ] Therefore leaf hash values change, requiring incremental trie updates

### 1.4 Blockifier Integration

- [ ] Update Blockifier dependency
  - [ ] Update to version supporting SNIP-34 and `get_compiled_class_hash_v2()`
  - [ ] Verify compatibility with Madara codebase
- [ ] Update Madara state adapters
  - [ ] Implement `get_compiled_class_hash_v2()` in `madara/crates/client/exec/src/blockifier_state_adapter.rs`
  - [ ] Implement `get_compiled_class_hash_v2()` in `madara/crates/client/exec/src/layered_state_adapter.rs`
  - [ ] Use migration mapping for fast lookups
  - [ ] Fallback to on-demand computation if mapping not found
- [ ] Update SNOS Blockifier code
  - [ ] Update SNOS state adapter to implement `get_compiled_class_hash_v2()`
  - [ ] Ensure SNOS can handle both Poseidon and BLAKE hashes
  - [ ] Update SNOS to use new Blockifier API
  - [ ] Test SNOS with updated Blockifier code

### 1.5 Block Production Updates

- [ ] Update sequencer declare transaction handling
  - [ ] Modify `madara/crates/client/block_production/src/executor/thread.rs`
  - [ ] Verify CASM hash uses correct function (BLAKE after activation)
  - [ ] Include migrated classes in state diff
- [ ] Update state diff creation
  - [ ] Modify `madara/crates/client/block_production/src/lib.rs`
  - [ ] Add `migrated_compiled_classes` to state diff

### 1.6 Full Node Sync Updates

- [ ] Update block import logic
  - [ ] Modify `madara/crates/client/sync/src/import.rs`
  - [ ] Verify CASM hashes based on block number
  - [ ] Handle migrated classes from gateway
- [ ] Update gateway parsing
  - [ ] Modify `madara/crates/client/sync/src/gateway/blocks.rs`
  - [ ] Parse `migrated_compiled_classes` from gateway

### 1.7 Cairo0 OS Code Updates

- [ ] **REMINDER**: Change default class-hash calculation back to BLAKE in cairo0 OS code
  - [ ] Revert Poseidon change that was made for 0.14.0 compatibility
  - [ ] Update cairo0 OS code to use BLAKE as default for v0.14.1
  - [ ] Verify cairo0 OS code compatibility with SNIP-34
  - [ ] Test cairo0 OS code with BLAKE hash

### 1.8 Testing

- [ ] Unit tests for BLAKE hash
- [ ] Unit tests for migration logic
- [ ] Integration tests for block production
- [ ] Integration tests for full node sync
- [ ] E2E tests for complete migration flow

## Phase 2: RPC 0.10.0 Implementation

### 2.1 Create v0.10.0 Module Structure

- [ ] Create RPC version directory
  - [ ] `madara/crates/client/rpc/src/versions/user/v0_10_0/`
  - [ ] Create `mod.rs`
  - [ ] Create `methods/` directory structure
- [ ] Copy base structure from v0.9.0
  - [ ] Copy `methods/read/` structure
  - [ ] Copy `methods/ws/` structure
  - [ ] Update imports and namespaces

### 2.2 State Diff Updates

- [ ] Add `migrated_compiled_classes` field
  - [ ] Update `madara/crates/primitives/state_update/src/lib.rs`
  - [ ] Add `MigratedClassItem` struct
  - [ ] Update serialization/deserialization
- [ ] Update state diff construction
  - [ ] Populate `migrated_compiled_classes` in all state diff creation points
  - [ ] Update block production state diff creation
  - [ ] Update sync state diff parsing

### 2.3 Preconfirmed State Update

- [ ] Remove `old_root` from `PreConfirmedStateUpdate`
  - [ ] Update `madara/crates/primitives/rpc/src/v0_10_0/mod.rs`
  - [ ] Update `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_state_update.rs`
  - [ ] Update OpenRPC schema
- [ ] Update RPC response construction
  - [ ] Remove `old_root` from preconfirmed responses
  - [ ] Keep `old_root` for confirmed blocks

### 2.4 Event Context Updates

- [ ] Add `transaction_index` and `event_index` to events
  - [ ] Update `madara/crates/primitives/block/src/lib.rs` (`EventWithInfo`)
  - [ ] Update event storage to include indices
  - [ ] Update event retrieval to return indices
- [ ] Update RPC event response
  - [ ] Modify `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_events.rs`
  - [ ] Include `transaction_index` and `event_index` in response
- [ ] Update event conversion
  - [ ] Ensure indices are populated during event creation
  - [ ] Update event serialization

### 2.5 Error Updates

- [ ] Add `CONTRACT_NOT_FOUND` error
  - [ ] Update `madara/crates/client/rpc/src/errors.rs`
  - [ ] Add error variant and serialization
- [ ] Update `estimate_fee`
  - [ ] Modify `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/estimate_fee.rs`
  - [ ] Add contract existence check
  - [ ] Return `CONTRACT_NOT_FOUND` when appropriate
- [ ] Update `estimate_message_fee`
  - [ ] Modify `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/estimate_message_fee.rs`
  - [ ] Add contract existence check
  - [ ] Return `CONTRACT_NOT_FOUND` when appropriate

### 2.6 Type Changes

- [ ] Update `storage_keys` parameter type
  - [ ] Check RPC spec for `STORAGE_KEY` definition
  - [ ] Update `madara/crates/primitives/rpc/src/v0_10_0/starknet_api_openrpc.json`
  - [ ] Update `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_storage_proof.rs`
  - [ ] Add `STORAGE_KEY` type definition if needed

### 2.7 Subscription Updates

- [ ] Update `subscribeReorg`
  - [ ] Modify `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/ws/subscribe_reorg.rs`
  - [ ] Add subscriptions for new transactions
  - [ ] Add subscriptions for new receipts
  - [ ] Fire reorg events on transaction/receipt updates
- [ ] Add backend subscription methods
  - [ ] Update `madara/crates/client/db/src/subscription.rs`
  - [ ] Add `subscribe_new_transactions()`
  - [ ] Add `subscribe_new_receipts()`
- [ ] Update block production notifications
  - [ ] Notify subscribers on transaction addition
  - [ ] Notify subscribers on receipt creation

### 2.8 OpenRPC Schema

- [ ] Update OpenRPC schema file
  - [ ] `madara/crates/primitives/rpc/src/v0_10_0/starknet_api_openrpc.json`
  - [ ] Add `migrated_compiled_classes` to state diff
  - [ ] Remove `old_root` from `PreConfirmedStateUpdate`
  - [ ] Add `transaction_index` and `event_index` to events
  - [ ] Add `CONTRACT_NOT_FOUND` error
  - [ ] Update `storage_keys` type
  - [ ] Update `subscribeReorg` description

### 2.9 RPC Module Registration

- [ ] Register v0.10.0 module
  - [ ] Update `madara/node/src/service/rpc/mod.rs`
  - [ ] Add v0.10.0 module to RPC server
  - [ ] Update version routing

### 2.10 Testing

- [ ] Unit tests for state diff updates
- [ ] Unit tests for preconfirmed state update
- [ ] Unit tests for event indices
- [ ] Unit tests for error handling
- [ ] Integration tests for RPC endpoints
- [ ] E2E tests for RPC compatibility

## Phase 3: Block Closing Improvements

### 3.1 Minimum Block Closing Interval

- [ ] Add minimum interval constant
  - [ ] Add `MIN_BLOCK_CLOSING_INTERVAL` constant
  - [ ] Set to 2 seconds
- [ ] Update block closing logic
  - [ ] Modify `madara/crates/client/block_production/src/executor/thread.rs`
  - [ ] Add minimum interval check
  - [ ] Track `last_block_close_time`
- [ ] Add activity detection
  - [ ] Implement `is_low_activity()` function
  - [ ] Track transaction activity
  - [ ] Update activity tracking on transaction addition

### 3.2 Configuration

- [ ] Add configuration option
  - [ ] Update `madara/crates/primitives/chain_config/src/lib.rs`
  - [ ] Add `min_block_closing_interval_seconds` field
- [ ] Add CLI parameter (optional)
  - [ ] Update `madara/node/src/cli/block_production.rs`
  - [ ] Add `min_block_closing_interval` parameter

### 3.3 Metrics and Logging

- [ ] Add metrics
  - [ ] Update `madara/crates/client/block_production/src/metrics.rs`
  - [ ] Add low activity block close counter
  - [ ] Add interval histogram
- [ ] Add logging
  - [ ] Log low activity block closes
  - [ ] Include timing information

### 3.4 Testing

- [ ] Unit tests for minimum interval logic
- [ ] Unit tests for activity detection
- [ ] Integration tests for block production
- [ ] E2E tests for low activity handling

## Phase 4: Integration and Testing

### 4.1 Integration

- [ ] Test SNIP-34 with RPC 0.10.0
- [ ] Test block closing with SNIP-34
- [ ] Test all features together
- [ ] Verify backward compatibility

### 4.2 Backward Compatibility

- [ ] Test v0.9.0 RPC endpoints still work
- [ ] Test pre-SNIP-34 blocks still work
- [ ] Test migration doesn't break existing data
- [ ] Verify configuration defaults

### 4.3 Performance Testing

- [ ] Benchmark BLAKE hash calculation
- [ ] Benchmark migration performance
- [ ] Test block closing performance
- [ ] Verify no regressions

### 4.4 Documentation

- [ ] Update code comments
- [ ] Update README if needed
- [ ] Document configuration options
- [ ] Document migration process

## Phase 5: Deployment Preparation

### 5.1 Pre-Deployment Checks

- [ ] All tests passing
- [ ] Code review completed
- [ ] Documentation complete
- [ ] Configuration validated
- [ ] Migration scripts tested

### 5.2 Deployment Plan

- [ ] Create deployment checklist
- [ ] Plan activation block for SNIP-34
- [ ] Plan RPC version rollout
- [ ] Plan monitoring and rollback

### 5.3 Monitoring

- [ ] Set up metrics dashboards
- [ ] Set up alerts for migration issues
- [ ] Monitor RPC endpoint usage
- [ ] Monitor block production performance

## Notes

- Implement changes incrementally
- Test each phase before moving to next
- Maintain backward compatibility throughout
- Document all changes thoroughly
- Coordinate with other node implementations
