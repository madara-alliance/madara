# Backward Compatibility Guide for v0.14.1

## Overview

This document outlines backward compatibility considerations for upgrading Madara from v0.14.0 to v0.14.1.

## Key Principles

1. **Data Compatibility**: All existing data must remain accessible
2. **API Compatibility**: Older RPC versions must continue to work
3. **Migration Safety**: Migrations must be reversible or non-destructive
4. **Gradual Rollout**: Changes should be activatable via configuration

## SNIP-34 Compatibility

### Pre-Activation Blocks

**Behavior**: All blocks before SNIP-34 activation use Poseidon hash for CASM.

**Requirements**:

- [ ] Verify CASM hashes using Poseidon for pre-activation blocks
- [ ] Store original Poseidon hashes (don't overwrite)
- [ ] Support reading both Poseidon and BLAKE hashes
- [ ] Migration mapping preserves old hashes

### Post-Activation Blocks

**Behavior**: All new declarations use BLAKE hash for CASM.

**Requirements**:

- [ ] Calculate CASM hashes using BLAKE after activation
- [ ] Store migration mapping (old → new hash)
- [ ] Include migrated classes in state diff
- [ ] Support reading migrated classes

### Migration Strategy

**Safe Migration Approach**:

1. **On-Demand Computation**: Compute BLAKE hashes when needed, not at startup
2. **Mapping Storage**: Store migration mappings separately (for caching)
3. **Dual Support**: Support both hash functions based on block number
4. **Gradual Activation**: Enable via configuration per chain
5. **Incremental Updates**: Update class trie incrementally as classes are accessed

**Implementation**:

```rust
// Check which hash function to use based on block number
fn should_use_blake(block_number: u64, activation_block: Option<u64>) -> bool {
    activation_block.map_or(false, |activation| block_number >= activation)
}

// Support both hash functions with on-demand computation
fn get_casm_hash(class_hash: Felt, block_number: u64) -> Result<Felt> {
    if should_use_blake(block_number, activation_block) {
        // Check cache first (previously computed hash)
        if let Some(migrated) = get_migration_mapping(class_hash) {
            return Ok(migrated.new_hash);
        }
        // Compute BLAKE hash on-demand
        let blake_hash = compute_blake_casm_hash(class_hash)?;
        // Cache the result
        store_migration_mapping(class_hash, old_hash, blake_hash)?;
        Ok(blake_hash)
    } else {
        // Use Poseidon hash
        get_poseidon_casm_hash(class_hash)
    }
}
```

### Database Schema Compatibility

**Current Schema**: Stores CASM hashes (Poseidon)

**New Schema**:

- Keep existing CASM hash storage (Poseidon)
- Add migration mapping table (separate)
- Add new CASM hash storage (BLAKE) for new classes

**Migration Table**:

```rust
pub struct MigratedClassHash {
    pub class_hash: Felt,
    pub old_compiled_class_hash: Felt,  // Poseidon
    pub new_compiled_class_hash: Felt,  // BLAKE
    pub migrated_at_block: u64,
}
```

**Benefits**:

- Original data preserved
- Can rollback if needed
- Supports both hash functions
- No data loss

## RPC Version Compatibility

### Version Support Strategy

**Supported Versions**:

- v0.7.1 (legacy)
- v0.8.1 (current)
- v0.9.0 (current)
- v0.10.0 (new)

**Default Version**: Can be configured (default to v0.9.0 initially, migrate to v0.10.0 later)

### Version-Specific Behavior

#### v0.9.0 (Pre-v0.14.1)

**State Update**:

- Includes `old_root` in `PreConfirmedStateUpdate`
- No `migrated_compiled_classes` in state diff
- Events don't include `transaction_index` and `event_index`

**Implementation**:

- Keep v0.9.0 implementation unchanged
- Use existing data structures
- No breaking changes

#### v0.10.0 (v0.14.1)

**State Update**:

- No `old_root` in `PreConfirmedStateUpdate`
- Includes `migrated_compiled_classes` in state diff
- Events include `transaction_index` and `event_index`

**Implementation**:

- New implementation alongside v0.9.0
- Uses updated data structures
- Includes all new features

### Client Migration Path

**Gradual Migration**:

1. Clients can continue using v0.9.0
2. Clients can migrate to v0.10.0 when ready
3. Both versions supported simultaneously
4. No forced migration

**Version Detection**:

```rust
// RPC version routing
match request.version {
    "0.9.0" => v0_9_0_handler.handle(request),
    "0.10.0" => v0_10_0_handler.handle(request),
    _ => error("Unsupported version"),
}
```

## Block Production Compatibility

### Block Closing Behavior

**Pre-v0.14.1**: Blocks close based on:

- Block time (configured)
- Block fullness
- Force close

**v0.14.1**: Adds:

- Minimum interval (2 seconds) for low activity

**Compatibility**:

- Existing block closing logic preserved
- New behavior additive (doesn't break existing)
- Can be disabled via configuration

**Configuration**:

```rust
pub struct ChainConfig {
    pub block_time_seconds: u64,                    // Existing
    pub min_block_closing_interval_seconds: Option<u64>, // New (optional)
}
```

**Behavior**:

- If `min_block_closing_interval_seconds` is `None`: Use old behavior
- If set: Use new behavior with minimum interval

## Data Migration Safety

### Migration Principles

1. **Non-Destructive**: Never delete or overwrite original data
2. **Reversible**: Can rollback if needed
3. **Incremental**: Migrate on-demand, not all at once
4. **Verifiable**: Can verify migration correctness

### CASM Hash Migration

**On-Demand Migration Process**:

1. **On First Access**: When class is accessed post-activation
2. **Read Original**: Read Poseidon hash from database
3. **Calculate New**: Calculate BLAKE hash from CASM class on-demand
4. **Store Mapping**: Cache old → new mapping for future use
5. **Update Trie**: Update class trie incrementally
6. **Use New**: Use BLAKE hash for new operations

**Rollback Strategy**:

- Keep original Poseidon hashes
- Keep migration mapping (cache)
- Can revert to Poseidon if needed
- No data loss
- No startup migration required

### Database Migration

**Migration Script**:

```rust
pub fn prepare_database_for_v0_14_1(
    backend: &MadaraBackend,
    activation_block: u64,
) -> Result<()> {
    // 1. Create migration mapping table (if not exists)
    //    This is used for caching computed BLAKE hashes
    backend.create_migration_table()?;

    // 2. No pre-computation - hashes computed on-demand
    //    Classes will be migrated as they are accessed

    // 3. Migration happens incrementally during normal operation
    //    No startup migration required

    Ok(())
}
```

**Safety Checks**:

- Verify migration mapping (cache) before use
- Check data integrity as classes are accessed
- On-demand approach naturally supports partial migrations
- No interruption risk (no startup migration)

## Testing Compatibility

### Test Strategy

1. **Pre-Activation Tests**: Test behavior before SNIP-34 activation
2. **Post-Activation Tests**: Test behavior after activation
3. **Migration Tests**: Test migration process
4. **Rollback Tests**: Test rollback capability

### Test Cases

#### RPC Compatibility Tests

- [ ] v0.9.0 endpoints return old format
- [ ] v0.10.0 endpoints return new format
- [ ] Both versions work simultaneously
- [ ] Version routing works correctly

#### Data Compatibility Tests

- [ ] Pre-activation blocks readable
- [ ] Post-activation blocks readable
- [ ] Migration mapping works
- [ ] Both hash functions supported

#### Migration Tests

- [ ] Migration doesn't break existing data
- [ ] Migration can be rolled back
- [ ] Migration is verifiable
- [ ] Migration is incremental

## Configuration Compatibility

### Configuration Defaults

**Safe Defaults**:

```rust
pub struct ChainConfig {
    // SNIP-34: Disabled by default (None = not activated)
    pub snip_34_activation_block: Option<u64> = None,

    // Block closing: Disabled by default (None = old behavior)
    pub min_block_closing_interval_seconds: Option<u64> = None,

    // RPC: Default to v0.9.0, can upgrade to v0.10.0
    pub default_rpc_version: RpcVersion = RpcVersion::V0_9_0,
}
```

**Activation**:

- SNIP-34: Set `snip_34_activation_block` to activate
- Block closing: Set `min_block_closing_interval_seconds` to activate
- RPC: Set `default_rpc_version` to v0.10.0

### Chain-Specific Configuration

**Different Chains**:

- Mainnet: Activate all features
- Testnet: Activate all features
- Devnet: Can enable/disable per chain
- Custom chains: Configurable

## Rollback Strategy

### Rollback Scenarios

1. **SNIP-34 Issues**: Revert to Poseidon hashes
2. **RPC Issues**: Revert to v0.9.0
3. **Block Closing Issues**: Disable minimum interval
4. **Migration Issues**: Rollback migration

### Rollback Procedures

#### SNIP-34 Rollback

1. Set `snip_34_activation_block` to `None`
2. Use Poseidon hashes for all operations
3. Migration mapping preserved (can re-migrate later)

#### RPC Rollback

1. Set `default_rpc_version` to v0.9.0
2. v0.10.0 endpoints still available but not default
3. Clients can switch back to v0.9.0

#### Block Closing Rollback

1. Set `min_block_closing_interval_seconds` to `None`
2. Revert to old block closing behavior
3. No data changes needed

## Monitoring Compatibility

### Metrics

**Track**:

- Migration progress
- Hash function usage (Poseidon vs BLAKE)
- RPC version usage
- Block closing behavior
- Error rates

**Alerts**:

- Migration failures
- Hash function mismatches
- RPC version errors
- Block closing issues

## Documentation

### User Documentation

- [ ] Document configuration options
- [ ] Document migration process
- [ ] Document rollback procedures
- [ ] Document compatibility guarantees

### Developer Documentation

- [ ] Document code changes
- [ ] Document data structures
- [ ] Document migration logic
- [ ] Document testing strategy

## Summary

**Compatibility Guarantees**:

1. ✅ All existing data remains accessible
2. ✅ Older RPC versions continue to work
3. ✅ Migrations are non-destructive
4. ✅ Changes are configurable and reversible
5. ✅ Gradual rollout supported

**Migration Safety**:

- Original data preserved
- Migration mapping stored separately
- Both hash functions supported
- Rollback capability maintained
