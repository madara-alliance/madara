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

### Known Differences: Gas & Fee Calculation

**IMPORTANT**: Fee calculations will differ between Rust and Blockifier. This is **expected and acceptable**.

**Why Fees Differ**:
- **Blockifier**: Charges for Cairo VM instruction execution (~827,885 gas for SimpleCounter)
  - Every Cairo instruction (memory ops, arithmetic, control flow) consumes gas
  - Accounts for the computational cost of running Cairo bytecode

- **Rust Native**: Only charges for observable operations (~5,900 gas for SimpleCounter)
  - Storage reads/writes
  - Event emissions
  - Signature verification
  - Execution is essentially "free" (microseconds)

**Impact**:
- Rust calculates fees ~140x lower than Blockifier
- ERC20 balance changes (fee transfers) will mismatch
- Example: SimpleCounter tx shows 82 Gwei difference

**What We Verify**:
✅ **Contract logic correctness** (storage updates, nonces, events)
✅ **Performance improvements** (90-140x faster than Cairo VM)
❌ Gas/fee parity (not a goal - defeats the purpose of native execution)

**Acceptable Mismatches**:
- ERC20 fee token balances (sender and sequencer)
- Gas consumption numbers

**Unacceptable Mismatches** (indicates bugs):
- Target contract storage updates
- Nonce increments
- Event content or ordering
- Execution success/failure status

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

---

## Gas & Fee Calculation - Known and Expected Differences

### Summary

**Status**: ✅ DOCUMENTED & ACCEPTED - Not a Bug

The Rust execution produces different gas/fee values compared to Blockifier. This is **expected and intentional** - it does not indicate incorrect contract logic.

### Why Fees Differ

**Blockifier (Cairo VM Execution)**:
- Tracks actual VM execution: steps, builtins (pedersen, ecdsa, keccak), memory holes
- Charges gas for Cairo instruction overhead (~10-30ms execution time)
- Example (StorageHeavy): 36.66ms execution → Higher gas → Higher fee

**Rust Native Execution**:
- Executes contract logic natively in Rust (~2ms execution time)
- Only observable operations matter (storage, events, state changes)
- Example (StorageHeavy): 2.12ms execution → Lower gas → Lower fee

**Performance Difference**: 17-282x speedup in Rust

### Impact on Verification

**ERC20 Balance Mismatches**:
- Rust charges LOWER fee → Sender pays less → Sender balance differs
- Rust transfers LOWER fee → Sequencer receives less → Sequencer balance differs

**Example (StorageHeavy tx: 0x0312a3f...)**:
```
Blockifier fee: ~X ETH (based on 36.66ms Cairo execution)
Rust fee: ~Y ETH (based on 2.12ms native execution, where Y < X)

Result:
- ERC20 sender balance: differs by (X - Y)
- ERC20 sequencer balance: differs by (X - Y)
```

### What We Verify (Contract Logic Correctness)

✅ **Storage Updates**: All contract storage writes match exactly (156/156 for StorageHeavy)
✅ **Events**: All emitted events match (count, keys, data) (10/10 for StorageHeavy)
✅ **Nonces**: Account nonce increments match
✅ **Execution Success/Failure**: Revert status matches
✅ **Return Data**: Function return values match

❌ **NOT Verified (Expected Differences)**:
- ERC20 fee token balances (gas calculation variance)
- Exact gas consumption values
- Fee amounts charged

### Why This Is Acceptable

1. **Contract Logic is Correct**: All state changes from contract execution match
2. **Performance Goal Achieved**: 17-282x faster than Cairo VM
3. **Simulating VM Overhead Defeats Purpose**: We want native speed, not VM simulation
4. **Fee Differences Are Predictable**: Always proportional to execution time difference

### Blockifier Gas Tracking Architecture

**Comprehensive Investigation**: Explored Blockifier, Sequencer, and CairoVM codebases

**Key Components**:
1. **GasVector** (`starknet_api/src/execution_resources.rs:106-187`):
   - `l1_gas`: Computation gas (VM steps, builtins)
   - `l1_data_gas`: Data availability gas (blobs/calldata)
   - `l2_gas`: Sierra gas (L2 execution)

2. **ExecutionResources** (`cairo-vm/vm/src/vm/runners/cairo_runner.rs:1580-1602`):
   - `n_steps`: Total VM steps executed
   - `n_memory_holes`: Memory gaps
   - `builtin_instance_counter`: HashMap of builtin usage counts

3. **Fee Calculation** (`starknet_api/src/execution_resources.rs:156-186`):
   ```
   Fee = (l1_gas * l1_gas_price)
       + (l1_data_gas * l1_data_gas_price)
       + (l2_gas * (l2_gas_price + tip))
   ```

**Gas Accumulation Flow**:
```
Cairo Execution
  ↓
VM tracks: steps, pedersen_count, ecdsa_count, keccak_count, etc.
  ↓
ExecutionResources extracted via runner.get_execution_resources()
  ↓
Blockifier converts resources to GasVector:
  - VM resources → l1_gas (via get_vm_resources_cost)
  - Archival data (events, calldata) → l2_gas
  - State changes → l1_gas (allocation costs)
  - DA segment → l1_data_gas
  ↓
GasVector.cost(gas_prices) → Final Fee
```

**Key Constants** (from `blockifier_versioned_constants_0_14_1.json`):
- VM step cost: `[25, 10000]` (25/10000 L1 gas per step)
- Pedersen: `[8, 100]` (8/100 L1 gas per hash)
- ECDSA: `[512, 100]` (512/100 L1 gas per signature)
- Keccak: `[512, 100]` (512/100 L1 gas per hash)
- Gas per data felt: `[5120, 1]` (5120 L2 gas per felt)
- Event key factor: `[2, 1]` (2x multiplier for event keys)

### Solution Options Considered

**Option A: Use Blockifier's Fee** (Recommended for parity):
- Extract fee from `execution_result.execution_info.receipt.fee`
- Pass to Rust execution: `executor.execute_invoke_with_fee(tx, account_class_hash, blockifier_fee)`
- Result: ERC20 balances match exactly
- Trade-off: Not verifying gas calculation algorithm

**Option B: Full Gas Replication** (Not recommended):
- Access `execution_result.execution_info.receipt.resources`
- Implement exact Blockifier gas formulas in Rust
- Use Rational arithmetic for precision matching
- Trade-off: Extremely complex, defeats native execution purpose

**Option C: Document & Accept** (Current approach):
- Document gas differences as expected behavior
- Focus verification on contract logic correctness
- Log gas mismatches as "EXPECTED" rather than "FAILED"
- Trade-off: ERC20 balances will differ, but contract logic is verified

### Verification Dumps

Results are dumped to `/Users/heemankverma/Work/Karnot/RvsC/verification_dumps/` for inspection:
- `<tx_hash>_blockifier.json`: Complete Blockifier execution result
- `<tx_hash>_rust.json`: Complete Rust execution result
- `<tx_hash>_comparison.txt`: Side-by-side comparison

### File Locations

**Gas Tracking Investigation**:
- Blockifier fee utils: `sequencer/crates/blockifier/src/fee/fee_utils.rs:67-111`
- GasVector definition: `sequencer/crates/starknet_api/src/execution_resources.rs:106-187`
- VM resource extraction: `sequencer/crates/blockifier/src/execution/entry_point_execution.rs:424-445`
- CairoVM resources: `cairo-vm/vm/src/vm/runners/cairo_runner.rs:1054-1076`
- Receipt calculation: `sequencer/crates/blockifier/src/fee/receipt.rs:50-121`
- Versioned constants: `sequencer/crates/blockifier/resources/blockifier_versioned_constants_0_14_1.json`

**Verification Code**:
- Full transaction verification: `madara/crates/client/exec/src/rust_exec_integration.rs:844-1100`
- Gas tracker: `madara/crates/client/rust-exec/src/gas.rs`
- Transaction executor: `madara/crates/client/rust-exec/src/transaction.rs`
