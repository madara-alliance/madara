# mc-rust-exec - Rust Native Contract Execution

## Purpose

This crate provides Rust-native implementations of Cairo contract functions that can run in parallel with Blockifier during transaction tracing. It produces the same outputs (state diff, events, return data) and compares them for verification.

## Design Decisions

### Location & Naming
- **Crate**: `mc-rust-exec` (follows Madara naming convention)
- **Location**: `madara/madara/crates/client/rust-exec/`

### Activation
- **Feature flag**: `rust-verification`
- Compile with: `cargo build --features rust-verification`
- Not enabled by default

### Mismatch Behavior
- **Log warning, return Blockifier result** (non-blocking)
- Rust execution failure does not break tracing
- Mismatches are logged for debugging

### Contract Organization
- Each contract has its own folder: `src/contracts/{contract_name}/`
- Identified by **name** (not class hash) for readability
- Class hash can be stored in the contract's `mod.rs` for verification

---

## Crate Structure

```
src/
├── lib.rs                    # Public API
├── types.rs                  # Core types (StateDiff, Event, ExecutionResult)
├── context.rs                # ExecutionContext - tracks state changes
├── state.rs                  # StateReader trait
├── storage.rs                # Storage key computation (sn_keccak, pedersen)
├── verify.rs                 # Comparison logic against Blockifier
│
└── contracts/                # Contract implementations
    ├── mod.rs                # Contract registry/dispatcher
    └── simple_counter/       # SimpleCounter contract
        ├── mod.rs            # Public interface + class hash
        ├── layout.rs         # Storage layout (variable keys)
        └── functions.rs      # Function implementations
```

---

## Public API

```rust
// Main entry point for trace_transaction integration
pub fn verify_transaction(
    state: &impl StateReader,
    contract_address: Felt,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller: Felt,
) -> Result<ExecutionResult, ExecutionError>;

// Compare Rust result with Blockifier result
pub fn compare_results(
    rust: &ExecutionResult,
    blockifier: &BlockifierExecutionInfo,
) -> VerificationResult;
```

---

## Integration with mc-exec

In `mc-exec/src/trace.rs`, the integration looks like:

```rust
#[cfg(feature = "rust-verification")]
{
    use mc_rust_exec::{verify_transaction, compare_results};

    // Run Rust execution with same state
    let rust_result = verify_transaction(
        &state_adapter,
        tx.contract_address,
        tx.class_hash,
        tx.entry_point_selector,
        &tx.calldata,
        tx.sender_address,
    );

    // Compare with Blockifier result
    if let Ok(rust_result) = rust_result {
        let verification = compare_results(&rust_result, &blockifier_result.execution_info);
        if !verification.passed {
            tracing::warn!(
                tx_hash = %tx_hash,
                mismatches = ?verification.errors,
                "Rust verification mismatch"
            );
        }
    }
}
```

---

## What Each Contract Folder Contains

### `layout.rs` - Storage Layout
```rust
pub struct SimpleCounterLayout {
    pub x_key: StorageKey,  // sn_keccak("x")
}
```

### `functions.rs` - Business Logic
```rust
pub fn execute_increment(
    state: &impl StateReader,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError>;
```

### `mod.rs` - Contract Metadata
```rust
pub const CLASS_HASH: Felt = felt!("0x...");
pub const NAME: &str = "SimpleCounter";

pub fn supports_selector(selector: Felt) -> bool;
pub fn execute(...) -> Result<ExecutionResult, ExecutionError>;
```

---

## Outputs Produced (Must Match Blockifier)

| Output | Description | Required |
|--------|-------------|----------|
| `state_diff.storage_updates` | Storage key-value changes | YES |
| `state_diff.address_to_nonce` | Nonce updates | YES |
| `retdata` | Function return values | YES |
| `events` | Emitted events (ordered) | YES |
| `failed` | Success/failure status | YES |
| `l2_to_l1_messages` | L1 messages | If applicable |
| `gas_consumed` | Gas used | Estimated (may differ) |

---

## Adding a New Contract

1. Create folder: `src/contracts/{contract_name}/`
2. Add `mod.rs` with class hash and dispatcher
3. Add `layout.rs` with storage key definitions
4. Add `functions.rs` with function implementations
5. Register in `src/contracts/mod.rs`

---

## Current Contracts

| Contract | Status | Functions |
|----------|--------|-----------|
| SimpleCounter | In Progress | `increment()` |

---

## Testing Strategy

1. **Unit tests**: Test individual functions with mock state
2. **Integration tests**: Run against real traces and compare
3. **Replay tests**: Replay historical transactions and verify

---

## Related Documents

See `/home/ubuntu/Work/RvsC/documents/` for:
- `rust_parallel_execution_for_tracing.md` - Architecture overview
- `rust_implementation_simple_example.md` - Detailed implementation guide
- `storage_key_and_crypto_operations.md` - How storage keys are computed
