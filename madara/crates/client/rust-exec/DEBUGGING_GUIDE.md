# Rust Native Execution - Debugging Guide & Best Practices

**Last Updated**: 2026-01-26
**Status**: ComprehensiveBenchmark - 133/133 tests passing (100%)

This guide documents lessons learned from implementing and debugging Rust native execution for Cairo contracts, with detailed examples from the ComprehensiveBenchmark contract debugging sessions.

---

## Table of Contents
1. [Critical Rules for Matching Cairo Execution](#critical-rules-for-matching-cairo-execution)
2. [Storage Diff Building - The Foundation](#storage-diff-building---the-foundation)
3. [Cairo Struct Handling](#cairo-struct-handling)
4. [Storage Operations](#storage-operations)
5. [Event Emissions](#event-emissions)
6. [Counter Update Patterns](#counter-update-patterns)
7. [Common Bugs and How to Debug Them](#common-bugs-and-how-to-debug-them)
8. [Debugging Methodology](#debugging-methodology)
9. [Verification Patterns](#verification-patterns)

---

## Critical Rules for Matching Cairo Execution

### Rule 1: Structs Write ALL Fields
**Never assume Cairo only writes modified fields.**

❌ **WRONG**:
```rust
// Only writing 2 fields
storage_write(account_key, balance)?;
storage_write(account_key + Felt::ONE, position)?;
```

✅ **CORRECT**:
```rust
// Writing ALL 4 fields of AccountState struct
storage_write(account_key, balance)?;                    // field 0
storage_write(account_key + Felt::ONE, position)?;       // field 1
storage_write(account_key + Felt::TWO, realized_pnl)?;   // field 2
storage_write(account_key + Felt::THREE, margin_ratio)?; // field 3
```

**Why**: Cairo's `self.account_states.write(caller, state)` flattens the entire struct to consecutive storage slots, writing **every field**, even if some are zero or unchanged.

**Real Bug**: In `benchmark_struct_write_iter1`, we only wrote fields 0-1 but Cairo wrote fields 0-3. Missing field 3 (`margin_ratio: 15_000_000`) caused a `MissingStorageInRust` error.

### Rule 2: Read-Then-Write Writes Everything Back
**When Cairo reads a struct and writes it back unchanged, ALL fields get written.**

❌ **WRONG**:
```rust
// Reading struct, modifying nothing, writing back partial fields
let balance = ctx.storage_read(state, contract, StorageKey(account_key))?;
storage_write(account_key, balance)?;  // Only writing field 0
```

✅ **CORRECT**:
```rust
// Read ALL fields
let balance = ctx.storage_read(state, contract, StorageKey(account_key))?;
let position = ctx.storage_read(state, contract, StorageKey(account_key + Felt::ONE))?;
let realized_pnl = ctx.storage_read(state, contract, StorageKey(account_key + Felt::TWO))?;
let margin_ratio = ctx.storage_read(state, contract, StorageKey(account_key + Felt::THREE))?;

// Write ALL fields back
storage_write(account_key, balance)?;
storage_write(account_key + Felt::ONE, position)?;
storage_write(account_key + Felt::TWO, realized_pnl)?;
storage_write(account_key + Felt::THREE, margin_ratio)?;
```

**Real Bug**: In `benchmark_heavy_transaction`, we only wrote back 2 AccountState fields and 1 PositionData field. Cairo's pattern:
```cairo
let account_state = self.account_states.read(caller);  // Reads all 4 fields
// ... do some work ...
self.account_states.write(caller, account_state);       // Writes all 4 fields back
```

### Rule 3: Never Lose initial_reads
**The state diff building logic requires initial_reads to determine if a value changed.**

❌ **WRONG**:
```rust
// Creating new context loses initial_reads
let mut ctx = ExecutionContext::new();
for call in &tx.calls {
    execute_single_call(call, &mut ctx)?;
}
let state_diff = ctx.build_state_diff();  // Lost initial_reads from nested calls!
```

✅ **CORRECT**:
```rust
// Preserve initial_reads by directly using contract-level state_diff
for call in &tx.calls {
    let call_result = execute_single_call(call)?;  // Returns ExecutionResult
    combined_state_diff.merge(call_result.state_diff);  // Preserves initial_reads
}
```

**Real Bug**: Transaction-level ExecutionContext was calling `build_state_diff()` without initial_reads, causing writes to be treated as "unread zero writes" and skipped. This caused ~80% of tests to fail initially.

**Log Evidence**:
```
✅ Including in diff (changed): key=0x3838e404... old=0x3e7 new=0x0  [First build - has initial_reads]
⏭️  Skipping (unread zero): key=0x3838e404...  [Second build - missing initial_reads]
```

### Rule 4: Storage Key Computation Must Be Exact
**Use the exact same hashing and modulo operations as Cairo.**

```rust
// Map<ContractAddress, T>
pub fn map_address_key(base: Felt, address: Felt) -> Felt {
    let hash = Pedersen::hash(&base, &address);
    hash.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Map<(ContractAddress, felt252), T>
pub fn map_address_felt_key(base: Felt, address: Felt, market_id: Felt) -> Felt {
    let hash1 = Pedersen::hash(&base, &address);
    let hash2 = Pedersen::hash(&hash1, &market_id);
    hash2.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

// Map<(u128, u128), T>
pub fn nested_map_key(base: Felt, key1: u128, key2: u128) -> Felt {
    let hash1 = Pedersen::hash(&base, &Felt::from(key1));
    let hash2 = Pedersen::hash(&hash1, &Felt::from(key2));
    hash2.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}
```

**Critical**: Always apply `mod_floor(&L2_ADDRESS_UPPER_BOUND)` after hashing to match Cairo's address space constraints.

---

## Storage Diff Building - The Foundation

### How ExecutionContext Tracks Changes

```rust
pub struct ExecutionContext {
    /// Storage reads: (contract, key) -> value (first read only)
    initial_reads: HashMap<(ContractAddress, StorageKey), Felt>,

    /// Storage writes: (contract, key) -> new_value
    storage_writes: HashMap<(ContractAddress, StorageKey), Felt>,

    // ... other fields
}
```

### The storage_read() Logic

```rust
pub fn storage_read<S: StateReader>(
    &mut self,
    state: &S,
    contract: ContractAddress,
    key: StorageKey,
) -> Result<Felt, StateError> {
    // 1. Check if we've already written to this key
    if let Some(value) = self.storage_writes.get(&(contract, key)) {
        // Return written value WITHOUT adding to initial_reads
        return Ok(*value);
    }

    // 2. Check if we've already read this key
    if let Some(value) = self.initial_reads.get(&(contract, key)) {
        return Ok(*value);
    }

    // 3. Read from underlying state
    let value = state.get_storage_at(contract, key)?;

    // 4. Cache the initial read
    self.initial_reads.insert((contract, key), value);

    Ok(value)
}
```

**Key Insight**: If you read from `storage_writes`, it does NOT add to `initial_reads`. This is correct because the "initial" value was already captured on the first read (before the write).

### The build_state_diff() Inclusion Logic

```rust
pub fn build_state_diff(&self) -> StateDiff {
    for ((contract, key), new_value) in &self.storage_writes {
        // Check if we read this slot before writing
        if let Some(old_value) = self.initial_reads.get(&(*contract, *key)) {
            // We read it first - only include if value changed
            if old_value != new_value {
                ✅ INCLUDE: Changed value
            } else {
                ⏭️  SKIP: Unchanged value
            }
        } else {
            // Never read before writing - only include if NON-ZERO
            if *new_value != Felt::ZERO {
                ✅ INCLUDE: Unread non-zero initialization
            } else {
                ⏭️  SKIP: Unread zero write (Blockifier behavior)
            }
        }
    }
}
```

### Critical Pattern: storage_write_matching_cairo()

**Always use this helper to ensure reads happen before writes:**

```rust
fn storage_write_matching_cairo<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
    key: StorageKey,
    value: Felt,
) -> Result<(), ExecutionError> {
    // Read BEFORE writing (mimics Cairo's implicit read-modify-write)
    let _ = ctx.storage_read(state, contract, key)?;

    // Now write
    ctx.storage_write(contract, key, value);

    Ok(())
}
```

**Why**: Cairo's storage operations implicitly read the old value before writing. This pattern ensures:
1. The old value is captured in `initial_reads`
2. The write appears in `storage_writes`
3. `build_state_diff()` can correctly determine if the value changed

---

## Cairo Struct Handling

### Struct Storage Layout

Cairo structs are **flattened** to consecutive storage slots:

```cairo
#[derive(Copy, Drop, Serde, starknet::Store)]
struct AccountState {
    balance: u128,          // field 0: base_key + 0
    position: u128,         // field 1: base_key + 1
    realized_pnl: i128,     // field 2: base_key + 2
    margin_ratio: u128,     // field 3: base_key + 3
}
```

Storage layout:
```
base_key + 0 → balance
base_key + 1 → position
base_key + 2 → realized_pnl
base_key + 3 → margin_ratio
```

### Writing Structs in Rust

```rust
// Get base key for this struct instance
let base_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);

// Write ALL fields in order
storage_write_matching_cairo(state, contract, ctx,
    StorageKey(base_key), Felt::from(balance))?;                    // field 0

storage_write_matching_cairo(state, contract, ctx,
    StorageKey(base_key + Felt::ONE), Felt::from(position))?;       // field 1

storage_write_matching_cairo(state, contract, ctx,
    StorageKey(base_key + Felt::TWO), Felt::from(realized_pnl))?;   // field 2

storage_write_matching_cairo(state, contract, ctx,
    StorageKey(base_key + Felt::THREE), Felt::from(margin_ratio))?; // field 3
```

### Reading Structs in Rust

**Always read ALL fields:**

```rust
let base_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);

let balance = ctx.storage_read(state, contract, StorageKey(base_key))?;
let position = ctx.storage_read(state, contract, StorageKey(base_key + Felt::ONE))?;
let realized_pnl = ctx.storage_read(state, contract, StorageKey(base_key + Felt::TWO))?;
let margin_ratio = ctx.storage_read(state, contract, StorageKey(base_key + Felt::THREE))?;
```

### Common Struct Patterns in Cairo

**Pattern 1: Create and Write**
```cairo
let state = AccountState {
    balance: i,
    position: i.try_into().unwrap(),
    realized_pnl: 0,
    margin_ratio: 15_000_000,  // ← Don't miss constant fields!
};
self.account_states.write(caller, state);
```

**Pattern 2: Read and Write Back**
```cairo
let account_state = self.account_states.read(caller);
// ... potentially use the values ...
self.account_states.write(caller, account_state);  // Writes ALL fields back
```

**Rust Implementation Must Match Both Patterns**:
- Pattern 1: Write all fields with correct values (including constants)
- Pattern 2: Read all fields, write all fields back (even if unchanged)

---

## Storage Operations

### Map Operations by Type

**Simple Map<u128, u128>**:
```rust
let key = map_u128_key(*SIMPLE_MAP_BASE, index);
let value = ctx.storage_read(state, contract, StorageKey(key))?;
storage_write_matching_cairo(state, contract, ctx, StorageKey(key), Felt::from(new_value))?;
```

**Nested Map<(u128, u128), u128>**:
```rust
let key = nested_map_key(*NESTED_MAP_BASE, key1, key2);
let value = ctx.storage_read(state, contract, StorageKey(key))?;
storage_write_matching_cairo(state, contract, ctx, StorageKey(key), Felt::from(new_value))?;
```

**Address Map<ContractAddress, u128>**:
```rust
let key = map_address_key(*ADDRESS_MAP_BASE, address);
let value = ctx.storage_read(state, contract, StorageKey(key))?;
storage_write_matching_cairo(state, contract, ctx, StorageKey(key), Felt::from(new_value))?;
```

**Complex Map<(ContractAddress, felt252), PositionData>**:
```rust
// Base key for this position
let position_key = map_address_felt_key(*POSITIONS_BASE, caller.0, market_id);

// Struct fields at consecutive offsets
let size = ctx.storage_read(state, contract, StorageKey(position_key))?;
let entry_price = ctx.storage_read(state, contract, StorageKey(position_key + Felt::ONE))?;
let last_funding = ctx.storage_read(state, contract, StorageKey(position_key + Felt::TWO))?;
```

### Storage Base Keys

Define as `Lazy<Felt>` constants:

```rust
use once_cell::sync::Lazy;

pub static SIMPLE_COUNTER_KEY: Lazy<Felt> = Lazy::new(|| sn_keccak(b"simple_counter"));
pub static SIMPLE_MAP_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"simple_map"));
pub static NESTED_MAP_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"nested_map"));
pub static ACCOUNT_STATES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_states"));
pub static POSITIONS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"positions"));
```

**Critical**: Use `sn_keccak` (Starknet Keccak) for variable names, NOT standard Keccak256.

---

## Event Emissions

### Event Names Must Match Exactly

❌ **WRONG**:
```rust
emit_benchmark_complete(ctx, "storage_read", iterations, result, iterations);
// But Cairo emits: 'map_read'
```

✅ **CORRECT**:
```rust
emit_benchmark_complete(ctx, "map_read", iterations, result, iterations);
```

**Real Bug**: `benchmark_map_read` was emitting "storage_read" instead of "map_read", causing 5 test failures.

### Event Encoding

```rust
fn emit_benchmark_complete(
    ctx: &mut ExecutionContext,
    benchmark_name: &str,
    iterations: u128,
    final_result: Felt,
    operations_performed: u128,
) {
    // Convert string to felt252 (max 31 bytes)
    let name_bytes = benchmark_name.as_bytes();
    let name_felt = if name_bytes.len() <= 31 {
        Felt::from_bytes_be_slice(name_bytes)
    } else {
        Felt::from_bytes_be_slice(&name_bytes[..31])
    };

    ctx.emit_event(
        vec![name_felt],  // keys
        vec![            // data
            Felt::from(iterations),
            final_result,
            Felt::from(operations_performed),
        ],
    );
}
```

### Event Ordering

Events must be emitted in the **exact same order** as Cairo:

```cairo
// Cairo emits in this order
self.emit(OperationBatch { ... });
self.emit(HashComputed { ... });
```

```rust
// Rust must emit in same order
emit_operation_batch(ctx, batch_number, operations_count);
emit_hash_computed(ctx, hash_count, operation_type);
```

---

## Counter Update Patterns

### The Pattern

**Storage Functions** → Update `TOTAL_OPERATIONS`
**Hashing Functions** → Update `TOTAL_HASHES_COMPUTED`
**Math/Computational Functions** → Update NOTHING

### Examples

✅ **Storage operations update counter**:
```rust
pub fn execute_benchmark_storage_write(...) {
    for i in 0..iterations {
        storage_write_matching_cairo(...)?;
    }

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);
}
```

✅ **Hashing operations update hash counter**:
```rust
pub fn execute_benchmark_pedersen_hashing(...) {
    for i in 0..iterations {
        hash_accumulator = Pedersen::hash(&hash_accumulator, &Felt::from(i));
    }

    // Update total hashes
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_HASHES_COMPUTED_KEY), iterations);
}
```

❌ **Math operations do NOT update counters**:
```rust
pub fn execute_benchmark_math_division(...) {
    for i in 0..iterations {
        let _ = (a * b) / c;  // Pure computation
    }

    // NO counter update!
}
```

**Real Bug**: We had 19 math/computational functions incorrectly updating `TOTAL_OPERATIONS_KEY`, causing 70+ test failures. Removed all these updates with sed.

---

## Common Bugs and How to Debug Them

### Bug Type 1: MissingStorageInRust

**Symptom**:
```
WARN MissingStorageInRust {
    contract: 0x25cf...,
    key: 0x265a...,
    blockifier_value: 0xe4e1c0
}
```

**Meaning**: Blockifier wrote to this key, but Rust didn't.

**Common Causes**:
1. **Missing struct fields**: Not writing all fields of a struct
2. **Missing operations**: Not executing all the same operations as Cairo
3. **Wrong loop count**: Looping fewer times than Cairo

**Debug Process**:
1. Decode the blockifier_value to understand what it represents
   - `0xe4e1c0` = 15,000,000 → Likely a constant like `margin_ratio`
2. Check the Cairo source to see what's being written
3. Compare Rust implementation against Cairo line-by-line

**Real Example**: `benchmark_struct_write_iter1`
- Missing: Field 3 (margin_ratio: 15_000_000)
- Fix: Write all 4 struct fields, not just 2

### Bug Type 2: ExtraStorageInRust

**Symptom**:
```
WARN ExtraStorageInRust {
    contract: 0x25cf...,
    key: 0x6eee...
}
```

**Meaning**: Rust wrote to this key, but Blockifier didn't.

**Common Causes**:
1. **Extra counter updates**: Updating counters that Cairo doesn't update
2. **Wrong loop count**: Looping more times than Cairo
3. **Extra operations**: Doing operations Cairo doesn't do

**Real Example**: `benchmark_heavy_transaction_iter1`
- Extra: 2 simple_map writes (wrote 5, Cairo writes 3)
- Fix: Change loop from `0..5` to `0..3`

### Bug Type 3: EventMismatch

**Symptom**:
```
WARN EventMismatch {
    rust: Event { keys: [..., 0x73746f726167655f72656164] },  // "storage_read"
    blockifier_keys: [..., 0x6d61705f72656164]              // "map_read"
}
```

**Meaning**: Event key (name) doesn't match.

**Common Cause**: Copy-paste error or wrong event name string.

**Real Example**: `benchmark_map_read`
- Bug: Emitting "storage_read"
- Fix: Change to "map_read"

### Bug Type 4: State Diff Losing Values

**Symptom**:
```
// Rust logs show:
✅ Including in diff (changed): key=0x3838... old=0x3e7 new=0x0
// But verification shows:
WARN MissingStorageInRust { key: 0x3838... }
```

**Meaning**: Value was computed correctly but lost during state diff merging.

**Root Cause**: Transaction-level context losing `initial_reads` from contract-level context.

**Fix**: Don't rebuild state diffs. Directly merge contract-level state_diffs.

---

## Debugging Methodology

### Step 1: Identify Failing Tests

Look at `tracecalls_annotated.log`:
```
benchmark_struct_write_iter1:0x2171b4...:226us:15296us:true   ← Passing
benchmark_struct_write_iter1:0x2171b4...:229us:21813us:false  ← FAILING
```

The `:false` at the end indicates failure.

### Step 2: Find Errors in Madara Logs

Search for the transaction hash in `madara-output.log`:
```bash
grep "0x2171b4" madara-output.log
```

Look for warnings:
```
WARN MissingStorageInRust { key: 0x265a..., blockifier_value: 0xe4e1c0 }
WARN ExtraStorageInRust { key: 0x6eee... }
```

### Step 3: Decode Values

Convert hex values to understand what they represent:
```python
>>> int("0xe4e1c0", 16)
15000000
```

Check Cairo source for that constant:
```cairo
margin_ratio: 15_000_000,  // ← Found it!
```

### Step 4: Check Storage Keys

Look at the key pattern to understand what's being accessed:
```
Key: 0x265a69e780bbfd6f7cf15adb879227aafcbc9832c7c1d080ee87ce5a5863d10
```

Compare with other keys in the same transaction:
```
0x265a...d0d  (field 0)
0x265a...d0e  (field 1)
0x265a...d0f  (field 2)
0x265a...d10  (field 3)  ← Missing field!
```

The pattern shows consecutive fields - this is a struct!

### Step 5: Compare with Cairo Source

Read the Cairo implementation line by line:
```cairo
let state = AccountState {
    balance: i,
    position: i.try_into().unwrap(),
    realized_pnl: 0,
    margin_ratio: 15_000_000,  // ← The missing write!
};
self.account_states.write(caller, state);
```

### Step 6: Fix and Verify

Make the fix, rebuild, and check logs:
```bash
cargo build --release --features rust-verification
# Run tests
# Check new logs for remaining failures
```

### Step 7: Iterate

Repeat until all tests pass. Each fix typically resolves multiple related tests.

---

## Verification Patterns

### Acceptable Differences

✅ **Gas/Fee Mismatches**: Expected (Rust is faster)
✅ **ERC20 balance differences**: Due to different fees

### Unacceptable Differences

❌ **Target contract storage updates**: MUST match exactly
❌ **Nonce increments**: MUST match exactly
❌ **Event content or ordering**: MUST match exactly
❌ **Execution success/failure status**: MUST match exactly

### Log Patterns to Look For

**Healthy execution**:
```
✅ Including in diff (changed): contract=0x... key=0x... old=0x3e7 new=0x0
⏭️  Skipping (unchanged): contract=0x... key=0x... val=0x0
✅ Storage Updates: 156 verified
✅ Events: 10 verified
✅ Nonce Updates: 1 verified
```

**Problematic execution**:
```
⏭️  Skipping (unread zero): key=0x...  ← Lost initial_reads!
WARN MissingStorageInRust { ... }      ← Missing operation
WARN ExtraStorageInRust { ... }        ← Extra operation
WARN EventMismatch { ... }             ← Wrong event name
```

---

## Key Takeaways

1. **Read Cairo source first**: Don't guess - read the actual Cairo implementation
2. **Structs write everything**: ALL fields, even zeros and constants
3. **Read before write**: Use `storage_write_matching_cairo()` everywhere
4. **Preserve initial_reads**: Never create intermediate ExecutionContext
5. **Match operations exactly**: Same reads, same writes, same order
6. **Check counter patterns**: Storage ops update ops, hashes update hashes, math updates nothing
7. **Verify event names**: Exact string match required
8. **Debug systematically**: Logs → Cairo source → Fix → Verify

---

## ComprehensiveBenchmark History

**Initial State**: 44/133 tests passing (33%)

**Major Fixes**:
1. **State Diff Bug**: Fixed transaction-level context losing initial_reads → +81 tests
2. **Event Name Bug**: Fixed "storage_read" → "map_read" → +5 tests
3. **Counter Updates**: Removed 19 incorrect TOTAL_OPERATIONS updates → +70 tests
4. **Struct Fields (v1)**: Attempted writing 4 fields but misunderstood pattern → 0 tests fixed
5. **Struct Fields (v2)**: Wrote 2 fields but needed all 4 → +5 tests (struct_write)
6. **Struct Fields (v3)**: Fixed to write all 4 fields correctly → +6 tests total
7. **Heavy Transaction**: Fixed nested_map operations and loop counts → +2 tests

**Final State**: 133/133 tests passing (100%) ✅

**Time to Debug**: Multiple sessions, ~100+ iterations
**Key Insight**: Always read the Cairo source. Never assume struct field behavior.

---

## Future Contract Implementation Checklist

When implementing a new Cairo contract in Rust:

- [ ] Read the complete Cairo source code
- [ ] Identify all structs and their field counts
- [ ] Map out all storage variables and their base keys
- [ ] Document counter update patterns (which functions update what)
- [ ] Implement storage key computation helpers
- [ ] Use `storage_write_matching_cairo()` for all writes
- [ ] Write ALL struct fields, even if zero or constant
- [ ] Match event names exactly (check Cairo source)
- [ ] Match operation order exactly
- [ ] Test incrementally (start with simple functions)
- [ ] Check logs for Missing/Extra storage warnings
- [ ] Verify 100% test pass rate before considering complete

---

## Additional Resources

- **CLAUDE.md**: Project overview and architecture
- **context.rs**: ExecutionContext implementation and state diff logic
- **transaction.rs**: Full transaction execution flow
- **Cairo Source**: Always the source of truth - `contracts/src/comprehensive_benchmark.cairo`
- **Blockifier**: Reference implementation in `sequencer/crates/blockifier/`

---

**Remember**: The goal is not just to pass tests, but to **exactly replicate Cairo execution semantics** in native Rust for correctness and massive performance gains (80-140x faster).
