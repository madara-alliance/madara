# Starknet v0.14.1 Upgrade Tasks

## Project Overview

Upgrade Madara from v0.14.0 to v0.14.1 compatibility, implementing:

- SNIP-34 (BLAKE CASM hash migration)
- RPC 0.10.0 specification
- DA encryption support
- Block closing improvements
- Maintaining backward compatibility

---

## [Madara] Full Node Support

### SNIP-34 CASM Hash Migration

#### BLAKE Hash Implementation

- [ ] Research and implement BLAKE hash calculation for CASM classes
- [ ] Create BLAKE hash computation function
- [ ] Add unit tests for BLAKE hash calculation

#### Migration Infrastructure

- [ ] Add SNIP-34 activation block configuration to chain config
- [ ] Create migration database schema for storing old→new hash mappings (for caching)
- [ ] Implement on-demand hash computation in `get_compiled_class_hash_v2()`
- [ ] Update class trie incrementally as classes are accessed

#### Blockifier Integration

- [ ] Update Blockifier dependency to version supporting SNIP-34
- [ ] Implement `get_compiled_class_hash_v2()` API support in Madara state adapters
- [ ] Update state adapter to compute BLAKE hashes on-demand
- [ ] Add caching mechanism for computed hashes
- [ ] Handle both Poseidon and BLAKE hashes based on block number

#### Full Node Sync

- [ ] Update block import to verify CASM hashes based on block number
- [ ] Handle migrated classes from gateway responses
- [ ] Parse and store `migrated_compiled_classes` from gateway
- [ ] Update gateway parsing to handle migrated classes

#### Cairo0 OS Code Updates

- [ ] **REMINDER**: Change default class-hash calculation back to BLAKE in cairo0 OS code
- [ ] Revert Poseidon change that was made for 0.14.0 compatibility
- [ ] Update cairo0 OS code to use BLAKE as default for v0.14.1
- [ ] Verify cairo0 OS code compatibility with SNIP-34

#### Testing

- [ ] Unit tests for BLAKE hash calculation
- [ ] Integration tests for migration process
- [ ] E2E tests for complete migration flow
- [ ] Tests for backward compatibility with pre-activation blocks
- [ ] Tests for full node sync with migrated classes

---

## [Madara] Sequencer Support

### SNIP-34 CASM Hash Migration

#### Block Production

- [ ] Update declare transaction handling to use BLAKE hash after activation
- [ ] Verify CASM hashes match expected values during block production
- [ ] Include migrated classes in state diff during block finalization
- [ ] Update state diff creation to include `migrated_compiled_classes`

#### Block Closing Improvements

- [ ] Add 2-second minimum block closing interval configuration
- [ ] Implement low activity detection logic
- [ ] Update block closing logic to use minimum interval
- [ ] Track transaction activity and timing
- [ ] Implement activity detection function
- [ ] Update block production state to track activity
- [ ] Add `min_block_closing_interval_seconds` to chain config
- [ ] Add optional CLI parameter for minimum interval
- [ ] Add metrics for low activity block closes
- [ ] Add logging for minimum interval block closes

#### Testing

- [ ] Unit tests for minimum interval logic
- [ ] Integration tests for block closing behavior
- [ ] Tests for activity detection
- [ ] Tests for block production with SNIP-34
- [ ] Tests for migrated classes in state diff

---

## [SNOS] 0.14.1 Support

### SNIP-34 CASM Hash Migration

#### Blockifier Integration

- [ ] Update Blockifier dependency to version supporting SNIP-34
- [ ] Update Blockifier code in SNOS to support `get_compiled_class_hash_v2()`
- [ ] Ensure SNOS state adapter implements `get_compiled_class_hash_v2()` function
- [ ] Update SNOS to handle both Poseidon and BLAKE hashes
- [ ] Ensure SNOS can work with updated Blockifier API

#### SNOS Input Updates

- [ ] Add public key field to SNOS input structure (`PieGenerationInput`)
- [ ] Update SNOS input generation to include public key
- [ ] Handle public key in SNOS job processing
- [ ] Ensure backward compatibility (empty public key for pre-v0.14.1)

#### Testing

- [ ] Unit tests for SNOS Blockifier integration
- [ ] Integration tests for SNOS with updated Blockifier code
- [ ] Tests for SNOS public key handling
- [ ] Tests for SNOS with both hash functions

---

## [Madara] RPC 0.10.0 Support

### RPC Module Structure

- [ ] Create v0.10.0 RPC module directory structure
- [ ] Copy base structure from v0.9.0
- [ ] Update OpenRPC schema file for v0.10.0
- [ ] Register v0.10.0 module in RPC server

### State Diff Updates

- [ ] Add `migrated_compiled_classes` field to StateDiff struct
- [ ] Populate `migrated_compiled_classes` during block finalization
- [ ] Ensure empty array in simulation/trace responses
- [ ] Update state diff serialization/deserialization

### Preconfirmed State Update

- [ ] Remove `old_root` field from `PreConfirmedStateUpdate` struct
- [ ] Update RPC response construction for preconfirmed blocks
- [ ] Keep `old_root` for confirmed blocks

### Event Context Enrichment

- [ ] Add `transaction_index` and `event_index` to event storage
- [ ] Update event retrieval to include indices
- [ ] Update RPC event response to include indices

### Error Handling

- [ ] Add `CONTRACT_NOT_FOUND` error variant
- [ ] Update `estimate_fee` to return `CONTRACT_NOT_FOUND` when appropriate
- [ ] Update `estimate_message_fee` to return `CONTRACT_NOT_FOUND` when appropriate

### Type Changes

- [ ] Update `storage_keys` parameter type from `FELT` to `STORAGE_KEY` in storage proof endpoint
- [ ] Add `STORAGE_KEY` type definition if needed

### WebSocket Subscriptions

- [ ] Update `subscribeReorg` to fire on new transactions
- [ ] Update `subscribeReorg` to fire on new receipts
- [ ] Add backend subscription methods for transactions and receipts

### Testing

- [ ] Unit tests for all RPC endpoint changes
- [ ] Integration tests for RPC compatibility
- [ ] Tests for backward compatibility with v0.9.0

---

## [Orchestrator] DA Encryption Support

### SNOS Input Public Key Support

- [ ] Add public key field to SNOS input structure (`PieGenerationInput`)
- [ ] Update SNOS input generation to include public key
- [ ] Handle public key in SNOS job processing
- [ ] Ensure backward compatibility (empty public key for pre-v0.14.1)

### Aggregator Input Public Key Support

- [ ] Add public key field to aggregator input structure
- [ ] Update aggregator input generation to include public key
- [ ] Coordinate with Herodotus for aggregator public key requirements
- [ ] Handle public key in aggregator job processing

### DA Encryption Implementation

- [ ] Implement DA encryption logic using public key
- [ ] Update orchestrator to encrypt DA data before submission
- [ ] Determine encryption stage (SNOS input vs aggregator input)
- [ ] Add encryption configuration options

### Configuration

- [ ] Add public key configuration to orchestrator config
- [ ] Add CLI parameters for public key
- [ ] Add environment variable support for public key
- [ ] Document public key requirements

### Testing

- [ ] Unit tests for public key handling
- [ ] Integration tests for DA encryption
- [ ] Tests for both SNOS and aggregator paths
- [ ] Tests for backward compatibility

---

## Integration and Compatibility

### Integration Testing

- [ ] Test SNIP-34 with RPC 0.10.0 together
- [ ] Test block closing with SNIP-34
- [ ] Test all features integrated
- [ ] Test SNOS with Madara full node
- [ ] Test SNOS with Madara sequencer

### Backward Compatibility

- [ ] Verify v0.9.0 RPC endpoints still work
- [ ] Verify pre-SNIP-34 blocks still work correctly
- [ ] Test migration doesn't break existing data
- [ ] Verify configuration defaults maintain compatibility

### Performance Testing

- [ ] Benchmark BLAKE hash calculation performance (on-demand)
- [ ] Test caching effectiveness for repeated class access
- [ ] Test block closing performance
- [ ] Verify no performance regressions
- [ ] Measure impact of on-demand computation on transaction simulation

### Documentation

- [ ] Update code comments
- [ ] Update README if needed
- [ ] Document configuration options
- [ ] Document migration process

---

## Deployment Preparation

### Pre-Deployment

- [ ] All tests passing
- [ ] Code review completed
- [ ] Documentation complete
- [ ] Configuration validated

### Deployment Plan

- [ ] Plan SNIP-34 activation block
- [ ] Plan RPC version rollout strategy
- [ ] Plan monitoring and alerting
- [ ] Plan rollback procedures

### Monitoring

- [ ] Set up metrics dashboards
- [ ] Set up alerts for migration issues
- [ ] Monitor RPC endpoint usage
- [ ] Monitor block production performance

---

## Notes

- Tasks should be implemented incrementally
- Test each component before moving to next
- Maintain backward compatibility throughout
- Coordinate with other node implementations
- **On-demand computation**: Hashes computed when needed, not at startup
- Caching ensures subsequent accesses are fast
