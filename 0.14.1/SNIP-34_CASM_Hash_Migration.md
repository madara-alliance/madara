# SNIP-34: CASM Hash Migration from Poseidon to BLAKE

## Overview

SNIP-34 changes the hash function used to calculate `compiled_class_hash` from **Poseidon** to **BLAKE**. This is a breaking change that requires careful migration of existing CASM hashes.

## Motivation

With the introduction of S-Two, many calculations can be re-optimized. Changing CASM hash calculation from Poseidon to BLAKE:

- Reduces block space requirements
- Improves gas efficiency
- Helps reach 80% gas target more consistently
- CASM hashes are now included in gas accounting

## Current Implementation

### CASM Hash Calculation (v0.14.0)

Currently, CASM hash is calculated using Poseidon hash in the `casm_classes_v2` crate:

**Location**: `madara/crates/primitives/class/src/compile.rs`

```rust
// Current implementation uses casm_classes_v2::CasmContractClass::compiled_class_hash()
// which internally uses Poseidon hash
let compiled_class_hash = casm_class.compiled_class_hash();
```

The hash is calculated in:

- `madara/crates/primitives/class/src/compile.rs:396` - v2 compiler
- `madara/crates/primitives/class/src/compile.rs:115,122,129` - older versions
- `madara/crates/client/db/src/rocksdb/update_global_trie/classes.rs:25` - class trie root calculation

### Class Trie Root Calculation

**Location**: `madara/crates/client/db/src/rocksdb/update_global_trie/classes.rs`

```rust
let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);
```

**IMPORTANT**: This does NOT change. The class trie root calculation continues to use Poseidon hash. This is separate from the CASM hash calculation itself. Pathfinder did not update this, and it's not mentioned in SNIP-34.

## Required Changes

### 0. Blockifier and Cairo0 OS Code Updates

**Blockifier Updates**:

- Update Blockifier dependency to version supporting SNIP-34
- Implement `get_compiled_class_hash_v2()` in Madara state adapters
- **CRITICAL**: Implement `get_compiled_class_hash_v2()` in SNOS state adapter
- Ensure both Madara and SNOS can handle Poseidon and BLAKE hashes

**Cairo0 OS Code**:

- **REMINDER**: Change default class-hash calculation back to BLAKE
- Revert Poseidon change that was made for 0.14.0 compatibility
- Update cairo0 OS code to use BLAKE as default for v0.14.1

### 1. Update CASM Hash Calculation to BLAKE

#### Option A: Update `casm_classes_v2` Dependency

If `casm_classes_v2` crate is updated to support BLAKE:

- Update dependency version
- Ensure `compiled_class_hash()` method uses BLAKE

#### Option B: Implement BLAKE Hash Locally

If `casm_classes_v2` doesn't support BLAKE yet, implement BLAKE hash calculation:

**New File**: `madara/crates/primitives/class/src/blake_casm_hash.rs`

```rust
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Blake2_256; // or appropriate BLAKE implementation

pub fn compute_blake_casm_hash(casm_class: &CasmContractClass) -> Felt {
    // Serialize CASM class to bytes
    // Calculate BLAKE hash
    // Return as Felt
}
```

**Update**: `madara/crates/primitives/class/src/compile.rs`

```rust
// Add feature flag or version check
#[cfg(feature = "blake_casm")]
let compiled_class_hash = compute_blake_casm_hash(&casm_class);
#[cfg(not(feature = "blake_casm"))]
let compiled_class_hash = casm_class.compiled_class_hash(); // Poseidon
```

### 2. Migration Strategy

#### Determine Activation Block

**Location**: `madara/crates/primitives/chain_config/src/lib.rs` (or similar)

Add configuration for SNIP-34 activation block:

```rust
pub struct ChainConfig {
    // ... existing fields
    pub snip_34_activation_block: Option<u64>, // None = not activated
}
```

#### CASM Hash Calculation with Version Check

**Location**: `madara/crates/primitives/class/src/compile.rs`

```rust
pub fn compile_to_casm(&self) -> Result<(Felt, CasmContractClass), ClassCompilationError> {
    let (compiled_class_hash, compiled_class) = match sierra_version {
        // ... existing versions
        _ => v2::compile(self)?,
    };

    // Check if SNIP-34 is active
    let compiled_class_hash = if should_use_blake(block_number) {
        compute_blake_casm_hash(&compiled_class)
    } else {
        compiled_class_hash // Use Poseidon
    };

    Ok((compiled_class_hash, compiled_class))
}
```

### 3. Migration of Existing CASM Hashes

#### Database Migration

**Location**: `madara/crates/client/db/src/lib.rs`

Existing classes in the database have CASM hashes calculated with Poseidon. These need to be migrated:

1. **Identify Classes to Migrate**: All Sierra classes declared before SNIP-34 activation
2. **Re-calculate CASM Hash**: Use BLAKE for classes declared after activation
3. **Store Migration Mapping**: Map old (Poseidon) hash to new (BLAKE) hash

**Performance**: On a 12-core CPU, recomputing CASM class hashes takes:

- ~17 seconds on Sepolia testnet
- ~8 seconds on mainnet

**Madara Approach**: Instead of pre-computing all hashes at startup, Madara uses **on-demand computation**. Hashes are computed when needed during transaction execution and cached for future use. This avoids startup delays while still providing fast access through caching.

**New Database Schema**:

```rust
// Add to database schema
pub struct MigratedClassHash {
    pub class_hash: Felt,
    pub old_compiled_class_hash: Felt,  // Poseidon hash
    pub new_compiled_class_hash: Felt,  // BLAKE hash
    pub migrated_at_block: u64,
}
```

#### Migration Function

**New File**: `madara/crates/client/db/src/migration/snip_34.rs`

**Note**: Madara uses **on-demand computation** instead of pre-computation. CASM hashes are computed when needed during transaction execution, not at startup.

```rust
pub fn migrate_class_trie_for_snip_34(
    backend: &MadaraBackend,
    activation_block: u64,
) -> Result<()> {
    tracing::info!("Preparing class trie for SNIP-34 migration...");

    // Note: We don't pre-compute BLAKE hashes here
    // Instead, we'll compute them on-demand during transaction execution
    // The class trie will be updated incrementally as classes are accessed

    // For now, we just need to ensure the migration infrastructure is ready
    // The actual trie updates happen when classes are accessed post-activation

    Ok(())
}
```

**On-Demand Computation Strategy**:

- CASM hashes are computed when `get_compiled_class_hash_v2()` is called
- Migration mapping is created and cached on first access
- Class trie is updated incrementally as classes are accessed
- No startup migration delay

### 4. Class Trie Root Calculation and Leaf Hash Updates

**Location**: `madara/crates/client/db/src/rocksdb/update_global_trie/classes.rs`

**CRITICAL**: While the leaf hash calculation **method** (Poseidon) doesn't change, the **compiled_class_hash value** does change (from Poseidon hash to BLAKE hash). This means:

1. **Leaf hash must be recomputed**: `leaf_hash = Poseidon::hash(CONTRACT_CLASS_HASH_VERSION, compiled_class_hash)`
2. **Class trie must be updated**: The class Merkle tree needs to be updated with new leaf hash values
3. **Otherwise**: You'll get a class commitment mismatch

**Current Implementation**:

```rust
pub fn class_trie_root(
    backend: &RocksDBStorage,
    declared_classes: &[DeclaredClassItem],
    block_number: u64,
) -> Result<Felt> {
    let mut class_trie = backend.class_trie();

    let updates: Vec<_> = declared_classes
        .into_par_iter()
        .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
            // compiled_class_hash is now BLAKE hash (after SNIP-34 activation)
            // But we still use Poseidon to hash it for the trie leaf
            let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);
            (*class_hash, hash)
        })
        .collect();

    // Insert/update leaf hashes in trie
    for (key, value) in updates {
        let bytes = key.to_bytes_be();
        let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        class_trie.insert(super::bonsai_identifier::CLASS, &bv, &value)?;
    }

    class_trie.commit(BasicId::new(block_number))?;
    Ok(class_trie.root_hash(super::bonsai_identifier::CLASS)?)
}
```

**Migration Requirement**: During migration, we need to:

1. **Recompute leaf hashes** for all migrated classes using new BLAKE `compiled_class_hash`
2. **Update class trie** with new leaf hash values
3. **Ensure trie consistency** - the class commitment must match the new compiled class hash

**Migration Function Update**:

```rust
pub fn migrate_casm_hashes_to_blake(
    backend: &MadaraBackend,
    activation_block: u64,
) -> Result<()> {
    tracing::info!("Starting CASM hash migration to BLAKE...");
    let start = Instant::now();

    // 1. Get all classes declared before activation_block
    let classes = backend.get_classes_before_block(activation_block)?;

    // 2. Pre-compute BLAKE hashes and migration mappings
    let mut migrated_classes = Vec::new();
    for class in classes {
        // Get CASM class definition
        let casm_class = backend.get_casm_class(&class.class_hash)?;

        // Calculate BLAKE hash
        let blake_hash = compute_blake_casm_hash(&casm_class)?;

        // Store migration mapping
        backend.store_migration_mapping(
            class.class_hash,
            class.compiled_class_hash, // Old Poseidon hash
            blake_hash,                // New BLAKE hash
        )?;

        migrated_classes.push(DeclaredClassItem {
            class_hash: class.class_hash,
            compiled_class_hash: blake_hash, // New BLAKE hash
        });
    }

    // 3. CRITICAL: Update class trie with new leaf hashes
    // This recomputes leaf hashes using new BLAKE compiled_class_hash values
    let class_trie_root = classes::class_trie_root(
        &backend.db,
        &migrated_classes,
        activation_block, // Use activation block number
    )?;

    // 4. Verify class commitment matches
    // The global state root should be recalculated with new class trie root

    let duration = start.elapsed();
    tracing::info!("CASM hash migration completed in {:?}", duration);

    Ok(())
}
```

**Important Notes**:

- The leaf hash calculation **method** (Poseidon) remains unchanged
- The **input** to the leaf hash (`compiled_class_hash`) changes from Poseidon to BLAKE
- Therefore, the **leaf hash values** change, requiring trie updates
- This must happen during migration, not just for new blocks

### 5. Block Production (Sequencer) Changes

**Location**: `madara/crates/client/block_production/src/lib.rs`

When producing blocks as a sequencer:

1. **Declare Transactions**: Use BLAKE for CASM hash after activation
2. **Class Verification**: Verify declared classes use correct hash function
3. **State Diff**: Include migrated classes in state diff

**Update**: `madara/crates/client/block_production/src/executor/thread.rs`

```rust
// When processing declare transactions
if should_use_blake(execution_state.exec_ctx.block_number) {
    // Verify compiled_class_hash uses BLAKE
    // Calculate BLAKE hash and compare
} else {
    // Use Poseidon verification
}
```

### 6. Full Node Sync Changes

**Location**: `madara/crates/client/sync/src/import.rs`

When syncing blocks from gateway:

1. **Verify CASM Hashes**: Use appropriate hash function based on block number
2. **Handle Migrated Classes**: Check migration mapping for old hashes
3. **Store Migrated Classes**: Store both old and new hashes if needed

**Update**: `madara/crates/client/sync/src/import.rs:344`

```rust
// Verify compiled class hash
if should_use_blake(block_n) {
    let expected_blake = compute_blake_casm_hash(&compiled_class);
    // Check against migration mapping if Poseidon hash provided
} else {
    // Use Poseidon verification
    if compiled_class_hash != sierra.compiled_class_hash {
        return Err(BlockImportError::CompiledClassHash { ... });
    }
}
```

### 7. State Diff Updates

**Location**: `madara/crates/primitives/state_update/src/lib.rs`

Add `migrated_compiled_classes` field to state diff (see RPC 0.10.0 changes):

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StateDiff {
    // ... existing fields
    pub migrated_compiled_classes: Vec<MigratedClassItem>, // NEW
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MigratedClassItem {
    pub class_hash: Felt,
    pub compiled_class_hash: Felt, // New BLAKE hash
}
```

**CRITICAL**: `migrated_compiled_classes` is **only populated when finalizing a block**, not during individual transaction execution.

- **During transaction simulation/estimation**: `migrated_compiled_classes` should be empty array `[]`
- **During trace operations**: `migrated_compiled_classes` should be empty array `[]`
- **During block finalization**: `migrated_compiled_classes` contains migrated classes

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

### 8. Blockifier Integration: `get_compiled_class_hash_v2`

**Location**:

- `madara/crates/client/exec/src/blockifier_state_adapter.rs`
- `madara/crates/client/exec/src/layered_state_adapter.rs`
- SNOS code (orchestrator)

Blockifier introduces a new API `get_compiled_class_hash_v2()` that handles both old (Poseidon) and new (BLAKE) class hashes.

**Important**: Both Madara and SNOS need to implement this function.

**Purpose**:

- Check if a transaction will trigger migration
- Determine if transaction can fit into block (resource limits)
- Handle both hash functions transparently

**Important Notes**:

- Migration resources don't affect transaction fee
- Migration resources are used to limit block resources
- Function is called to check if transaction fits into block

**Implementation**:

```rust
impl BlockifierStateAdapter {
    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        block_number: u64,
    ) -> StateResult<CompiledClassHash> {
        // Check if SNIP-34 is active
        if should_use_blake(block_number) {
            // Check if we already computed and cached this hash
            if let Some(migrated) = self.get_migration_mapping(class_hash) {
                return Ok(CompiledClassHash(migrated.new_hash));
            }

            // Compute BLAKE hash on-demand
            let casm_class = self.get_casm_class(class_hash)?;
            let blake_hash = compute_blake_casm_hash(&casm_class)?;

            // Cache the result for future use
            self.store_migration_mapping(
                class_hash,
                self.get_old_compiled_class_hash(class_hash)?,
                blake_hash,
            )?;

            Ok(CompiledClassHash(blake_hash))
        } else {
            // Use Poseidon hash for pre-activation blocks
            self.get_compiled_class_hash(class_hash)
        }
    }

    fn get_migration_mapping(&self, class_hash: ClassHash) -> Option<MigratedClassHash> {
        // Check cache/database for previously computed hash
        self.view.backend().get_migration_mapping(&class_hash.to_felt()).ok().flatten()
    }
}
```

**SNOS Implementation**:

SNOS also needs to implement `get_compiled_class_hash_v2()` in its state adapter:

**Location**: SNOS code (orchestrator)

```rust
// SNOS state adapter implementation
impl SnosStateAdapter {
    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        block_number: u64,
    ) -> StateResult<CompiledClassHash> {
        // Similar implementation to Madara
        // Check if SNIP-34 is active
        if should_use_blake(block_number) {
            // Check cache first
            if let Some(migrated) = self.get_migration_mapping(class_hash) {
                return Ok(CompiledClassHash(migrated.new_hash));
            }

            // Compute on-demand
            let casm_class = self.get_casm_class(class_hash)?;
            let blake_hash = compute_blake_casm_hash(&casm_class)?;

            // Cache result
            self.store_migration_mapping(class_hash, old_hash, blake_hash)?;

            Ok(CompiledClassHash(blake_hash))
        } else {
            self.get_compiled_class_hash(class_hash)
        }
    }
}
```

### 9. On-Demand Computation Strategy

**Madara Approach**: Compute BLAKE hashes on-demand during transaction execution, not at startup.

**Rationale**:

- Avoids startup delay (~8-17 seconds)
- Computes hashes only when needed
- Caches computed hashes for future use
- Simpler migration process

**Implementation**:

```rust
impl BlockifierStateAdapter {
    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        block_number: u64,
    ) -> StateResult<CompiledClassHash> {
        // Check if SNIP-34 is active
        if should_use_blake(block_number) {
            // Check if we already computed and cached this hash
            if let Some(migrated) = self.get_migration_mapping(class_hash) {
                return Ok(CompiledClassHash(migrated.new_hash));
            }

            // Compute BLAKE hash on-demand
            let casm_class = self.get_casm_class(class_hash)?;
            let blake_hash = compute_blake_casm_hash(&casm_class)?;

            // Cache the result for future use
            self.store_migration_mapping(
                class_hash,
                self.get_old_compiled_class_hash(class_hash)?, // Old Poseidon hash
                blake_hash,                                    // New BLAKE hash
            )?;

            Ok(CompiledClassHash(blake_hash))
        } else {
            // Use Poseidon hash for pre-activation blocks
            self.get_compiled_class_hash(class_hash)
        }
    }

    fn get_migration_mapping(&self, class_hash: ClassHash) -> Option<MigratedClassHash> {
        // Check cache/database for previously computed hash
        self.view.backend().get_migration_mapping(&class_hash.to_felt()).ok().flatten()
    }
}
```

**Performance Considerations**:

- First access to a class will compute the hash (may be slower)
- Subsequent accesses use cached hash (fast)
- Migration mapping is stored in database for persistence
- Class trie is updated incrementally as classes are accessed

**Benefits**:

- No startup delay (unlike pre-computation approach)
- Computes hashes only when needed
- Caching ensures fast subsequent accesses
- Simpler implementation

### 10. Transaction Simulation and Estimation

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

### 11. RPC Changes

**Location**: `madara/crates/client/rpc/src/versions/user/v0_10_0/`

Include `migrated_compiled_classes` in state update responses (see RPC 0.10.0 documentation).

**Important**:

- Empty array `[]` in simulation/trace responses
- Populated only in finalized block state updates

## Testing Requirements

### Unit Tests

1. **BLAKE Hash Calculation**: Test BLAKE hash matches expected values
2. **Migration Logic**: Test migration mapping works correctly
3. **Version Detection**: Test correct hash function is used based on block number

### Integration Tests

1. **Block Production**: Test sequencer produces blocks with correct CASM hashes
2. **Full Node Sync**: Test syncing blocks with migrated classes
3. **RPC Responses**: Test state updates include migrated classes

### E2E Tests

1. **Migration Flow**: Test complete migration from Poseidon to BLAKE
2. **Backward Compatibility**: Test old blocks still work correctly
3. **Cross-Node Compatibility**: Test compatibility with other node implementations

## Backward Compatibility

### Pre-Activation Blocks

- All blocks before SNIP-34 activation use Poseidon
- No migration needed for historical data
- Verification uses Poseidon for pre-activation blocks

### Post-Activation Blocks

- All new declarations use BLAKE
- Migration mapping stores old→new hash relationships
- State diffs include migrated classes for transparency

## Performance Considerations

1. **On-Demand Computation**: Compute BLAKE hashes when needed during transaction execution
2. **Caching**: Cache computed hashes in migration mapping for future use
3. **Incremental Updates**: Update class trie incrementally as classes are accessed
4. **Lazy Migration**: Migration happens naturally as classes are used, not at startup

## Important Reminders

### Cairo0 OS Code

**CRITICAL REMINDER**: The default class-hash calculation in cairo0 OS code was changed to Poseidon for 0.14.0 compatibility. For v0.14.1, this must be changed back to BLAKE.

**Action Required**:

- Revert Poseidon change in cairo0 OS code
- Update to use BLAKE as default
- Verify compatibility with SNIP-34

### SNOS Updates

SNOS requires additional updates beyond Madara:

- Implement `get_compiled_class_hash_v2()` in SNOS state adapter
- Update SNOS to handle both Poseidon and BLAKE hashes
- Ensure SNOS can work with updated Blockifier API
- Test SNOS with updated Blockifier code

## References

- [SNIP-34 Specification](https://community.starknet.io/t/snip-34-more-efficient-casm-hashes/115979)
- [Pathfinder PR #3094](https://github.com/eqlabs/pathfinder/pull/3094) - BLAKE CASM pre-computation
