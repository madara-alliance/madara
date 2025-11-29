# Pathfinder Implementation Analysis for SNIP-34

## Overview

This document analyzes Pathfinder's implementation approach for SNIP-34 (BLAKE CASM hash migration) based on their GitHub PRs and community discussions. Pathfinder is a full-node implementation without sequencer capabilities, so their approach differs from Madara's requirements.

## Key PRs Analyzed

1. **PR #3068**: RPC 0.10.0 state-update changes
2. **PR #3091**: Adding support for migrated classes
3. **PR #3094**: Pre-compute of CASM classes for BLAKE

## Important Clarifications from Community Discussions

### 1. Class Trie Root Calculation and Leaf Hash Updates

**Finding**: While the leaf hash calculation **method** (Poseidon) doesn't change, the **compiled_class_hash value** does change (from Poseidon to BLAKE). This means:

1. **Leaf hash must be recomputed**: `leaf_hash = Poseidon::hash(CONTRACT_CLASS_HASH_VERSION, compiled_class_hash)`
2. **Class trie must be updated**: The class Merkle tree needs to be updated with new leaf hash values
3. **Otherwise**: You'll get a class commitment mismatch

**Current Implementation** (Madara):

```rust
// madara/crates/client/db/src/rocksdb/update_global_trie/classes.rs
let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);
```

**Key Insight**:

- The hash function (Poseidon) stays the same
- But the input (`compiled_class_hash`) changes from Poseidon hash to BLAKE hash
- Therefore, the output (leaf hash) changes
- The class trie must be updated with new leaf hash values

**Action**:

- Keep the hash function unchanged (Poseidon)
- But recompute leaf hashes using new BLAKE `compiled_class_hash` values
- Update the class trie during migration

### 2. Migration Performance

**Performance Metrics**:

- **Sepolia testnet**: ~17 seconds on 12-core CPU
- **Mainnet**: ~8 seconds on 12-core CPU

**Implication**:

- Single migration step at startup is feasible
- No need for incremental/on-demand migration
- Can recompute all CASM hashes during node startup

**Migration Strategy**:

```rust
// Migration can be done synchronously at startup
pub fn migrate_all_casm_hashes_to_blake(
    backend: &MadaraBackend,
    activation_block: u64,
) -> Result<()> {
    // Get all classes declared before activation_block
    // Recompute CASM hash using BLAKE
    // Store migration mapping
    // This takes ~8-17 seconds, acceptable for startup
}
```

### 3. Migrated Classes in State Diff

**Key Finding**: `migrated_compiled_classes` is only populated when **finalizing a block**, not during individual transaction execution.

**Implications**:

- During transaction simulation/estimation: `migrated_compiled_classes` should be empty array
- During trace operations: `migrated_compiled_classes` should be empty array
- Only in finalized block state updates: `migrated_compiled_classes` contains migrated classes

**Implementation**:

```rust
// During transaction execution (simulation/trace)
pub fn get_state_diff_for_transaction(
    execution_info: &TransactionExecutionInfo,
) -> StateDiff {
    StateDiff {
        // ... other fields
        migrated_compiled_classes: vec![], // EMPTY during transaction execution
    }
}

// During block finalization
pub fn get_state_diff_for_block(
    block_execution_summary: &BlockExecutionSummary,
) -> StateDiff {
    StateDiff {
        // ... other fields
        migrated_compiled_classes: block_execution_summary.migrated_classes.clone(),
    }
}
```

### 4. Blockifier API Changes

**New API**: `get_compiled_class_hash_v2()`

**Purpose**:

- Handles both old (Poseidon) and new (BLAKE) class hashes
- Used to check if a transaction will trigger migration
- Used to determine if transaction can fit into block (resource limits)

**Key Points**:

- Migration resources don't affect transaction fee
- Migration resources are used to limit block resources
- Function is called to check if transaction fits into block

**Implementation Requirements**:

```rust
// Blockifier state adapter needs to support v2 API
impl BlockifierStateAdapter {
    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        block_number: u64,
    ) -> StateResult<CompiledClassHash> {
        // Check if SNIP-34 is active
        if should_use_blake(block_number) {
            // Check migration mapping first
            if let Some(migrated) = self.get_migration_mapping(class_hash) {
                return Ok(CompiledClassHash(migrated.new_hash));
            }
            // Calculate BLAKE hash if not migrated
            let blake_hash = self.compute_blake_casm_hash(class_hash)?;
            Ok(CompiledClassHash(blake_hash))
        } else {
            // Use Poseidon hash for pre-activation blocks
            self.get_compiled_class_hash(class_hash)
        }
    }
}
```

### 5. Pre-computation Strategy

**Pathfinder Approach**: Pre-compute and store CASM class hashes in database to avoid computational cost during simulation.

**Why Pre-computation is Necessary**:

Based on community discussions, the key reasons are:

1. **Transaction Simulation Performance**:
   - During transaction simulation/estimation, Blockifier needs to determine if a transaction will trigger migration
   - This requires calling `get_compiled_class_hash_v2()` for each declared class
   - Calculating BLAKE hash on-demand during simulation is computationally expensive
   - Pre-computing allows fast database lookup instead of expensive computation

2. **Non-blocking Simulation**:
   - Transaction simulation/estimation should be fast and non-blocking
   - Calculating BLAKE hash synchronously during simulation would block the RPC response
   - Pre-computation moves the expensive work to startup (one-time cost)

3. **Resource Efficiency**:
   - CASM hash calculation is expensive (requires serializing CASM class and computing hash)
   - Pre-computing once at startup is more efficient than computing repeatedly during simulation
   - Database lookup is orders of magnitude faster than hash computation

4. **User Experience**:
   - Faster simulation/estimation responses improve user experience
   - Predictable performance (no spikes during simulation)
   - Consistent response times

5. **Blockifier Requirements**:
   - Blockifier's `get_compiled_class_hash_v2()` is called during transaction execution
   - It needs to check if a transaction will trigger migration (resource limits)
   - Fast lookup is essential for efficient block production

**From Community Discussion**:

- **Krisztian Kovacs** suggested pre-computing and storing CASM class hashes to avoid computational cost during simulation
- **Rodrigo** agreed that calculating hashes ahead of time is necessary
- The consensus was that on-demand calculation would be too slow for production use

**Implementation**:

```rust
// Pre-compute BLAKE hashes for all classes
pub fn precompute_blake_casm_hashes(
    backend: &MadaraBackend,
    activation_block: u64,
) -> Result<()> {
    // Get all Sierra classes
    let classes = backend.get_all_sierra_classes()?;

    // Pre-compute BLAKE hashes
    for class in classes {
        let blake_hash = compute_blake_casm_hash(&class)?;

        // Store in migration mapping
        backend.store_migration_mapping(
            class.class_hash,
            class.compiled_class_hash, // Old Poseidon hash
            blake_hash,                // New BLAKE hash
        )?;
    }

    Ok(())
}

// Use pre-computed hash during simulation
pub fn get_compiled_class_hash_for_simulation(
    &self,
    class_hash: ClassHash,
) -> StateResult<CompiledClassHash> {
    // Fast database lookup
    if let Some(migrated) = self.get_migration_mapping(class_hash) {
        return Ok(CompiledClassHash(migrated.new_hash));
    }

    // Fallback: compute on-demand (should be rare)
    self.compute_blake_casm_hash(class_hash)
}
```

### 6. Transaction Simulation and Estimation

**Important**: Migration detection happens during transaction execution, not during simulation.

**Flow**:

1. Transaction is simulated/estimated
2. Blockifier calls `get_compiled_class_hash_v2()` for each declared class
3. If class hash differs from stored hash, migration is detected
4. Migration resources are calculated but don't affect fee
5. Migration resources are used to check if transaction fits in block

**Implementation**:

```rust
// During transaction simulation
pub async fn simulate_transaction(
    &self,
    transaction: Transaction,
) -> Result<SimulationResult> {
    // Execute transaction
    let (execution_info, state_maps) = executor.execute(transaction)?;

    // migrated_compiled_classes is NOT available here
    // It's only populated during block finalization

    Ok(SimulationResult {
        execution_info,
        state_maps,
        // migrated_compiled_classes: vec![], // Empty during simulation
    })
}
```

## Pathfinder's Implementation Details

### Database Schema

**Migration Mapping Table**:

```rust
pub struct MigratedClassHash {
    pub class_hash: Felt,
    pub old_compiled_class_hash: Felt,  // Poseidon
    pub new_compiled_class_hash: Felt,  // BLAKE
    pub migrated_at_block: u64,
}
```

**Storage**:

- Separate column family for migration mappings
- Indexed by `class_hash` for fast lookup
- Indexed by `old_compiled_class_hash` for reverse lookup

### CASM Hash Calculation

**BLAKE Hash Implementation**:

- Uses BLAKE2s (256-bit output)
- Serializes CASM class to bytes
- Calculates hash over serialized bytes
- Converts to Felt

**Code Location** (Pathfinder):

- `pathfinder/crates/storage/src/class_hash.rs`
- `pathfinder/crates/storage/src/migration.rs`

### RPC Changes

**State Update Response**:

- Added `migrated_compiled_classes` field
- Only populated in finalized block state updates
- Empty array in simulation/trace responses

**Preconfirmed State Update**:

- Removed `old_root` field (as per RPC 0.10.0 spec)

## Differences from Madara

### 1. Sequencer Capabilities

**Pathfinder**: Full-node only, no sequencer
**Madara**: Both full-node and sequencer

**Implications**:

- Madara needs to handle migration during block production
- Madara needs to include migrated classes in state diff during block finalization
- Madara needs to verify CASM hashes during transaction validation

### 2. Block Production

**Pathfinder**: Only syncs blocks from gateway
**Madara**: Produces blocks as sequencer

**Additional Requirements for Madara**:

- Calculate BLAKE hash for new declare transactions
- Include migrated classes in state diff when finalizing blocks
- Verify CASM hashes match expected values

### 3. Migration Timing

**Pathfinder**: Migration happens during sync
**Madara**: Migration happens:

- During startup (pre-computation)
- During block production (new declarations)
- During sync (from gateway)

## Recommendations for Madara

### 1. Migration Strategy

**Madara Approach - On-Demand Computation**:

- Compute CASM hashes on-demand during transaction execution
- Cache computed hashes in migration mapping
- **CRITICAL**: Update class trie incrementally as classes are accessed
- No startup migration delay

**Note**: Pathfinder uses pre-computation, but Madara uses on-demand computation for simplicity and to avoid startup delays.

**Implementation** (On-Demand Approach):

```rust
impl BlockifierStateAdapter {
    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        block_number: u64,
    ) -> StateResult<CompiledClassHash> {
        if should_use_blake(block_number) {
            // Check cache first
            if let Some(migrated) = self.get_migration_mapping(class_hash) {
                return Ok(CompiledClassHash(migrated.new_hash));
            }

            // Compute on-demand
            let casm_class = self.get_casm_class(class_hash)?;
            let blake_hash = compute_blake_casm_hash(&casm_class)?;

            // Cache result
            self.store_migration_mapping(
                class_hash,
                self.get_old_compiled_class_hash(class_hash)?,
                blake_hash,
            )?;

            // Update class trie incrementally
            self.update_class_trie_for_class(class_hash, blake_hash)?;

            Ok(CompiledClassHash(blake_hash))
        } else {
            self.get_compiled_class_hash(class_hash)
        }
    }
}
```

**Key Points**:

- No startup migration - hashes computed on-demand
- Class trie updated incrementally as classes are accessed
- Cached results avoid recomputation
- Simpler implementation than pre-computation approach

### 2. Blockifier Integration

**Update State Adapter**:

- Implement `get_compiled_class_hash_v2()` if Blockifier provides it
- Use migration mapping for fast lookups
- Fallback to computation if mapping not found

### 3. State Diff Population

**During Block Finalization**:

- Collect migrated classes during block execution
- Include in state diff when finalizing block
- Empty array during simulation/trace

**During Simulation/Trace**:

- Always return empty `migrated_compiled_classes`
- Don't populate during individual transaction execution

### 4. Performance Optimization

**On-Demand Computation** (Madara):

- Compute BLAKE hashes when needed during transaction execution
- Cache computed hashes in migration mapping
- Update class trie incrementally as classes are accessed
- No startup delay

**Caching**:

- Cache migration mappings in memory
- Use database as persistent storage
- Invalidate cache on reorg

## Testing Considerations

### 1. Migration Tests

- Test startup migration completes successfully
- Test migration mapping is stored correctly
- Test reverse lookup (old hash → new hash)

### 2. Block Production Tests

- Test new declare transactions use BLAKE hash
- Test migrated classes included in state diff
- Test CASM hash verification

### 3. Full Node Sync Tests

- Test syncing blocks with migrated classes
- Test parsing `migrated_compiled_classes` from gateway
- Test state diff includes migrated classes

### 4. RPC Tests

- Test state update includes migrated classes (finalized blocks)
- Test state update has empty migrated classes (simulation)
- Test preconfirmed state update doesn't have `old_root`

## References

- [Pathfinder PR #3068](https://github.com/eqlabs/pathfinder/pull/3068) - RPC 0.10.0 state-update changes
- [Pathfinder PR #3091](https://github.com/eqlabs/pathfinder/pull/3091) - Migrated classes support
- [Pathfinder PR #3094](https://github.com/eqlabs/pathfinder/pull/3094) - BLAKE CASM pre-computation
- [SNIP-34 Specification](https://community.starknet.io/t/snip-34-more-efficient-casm-hashes/115979)
