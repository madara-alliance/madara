# POC Plan: Parallel Global Trie Calculation

## Scope
Build a POC binary in Madara that:
- Computes state roots sequentially (baseline).
- Computes state roots in parallel using multiple DB copies (prefix strategy).
- Outputs timing + correctness info to JSON for AI analysis.

## How to Run (End-to-End)

### 0) Prereqs
- Madara devnet DB path (e.g. `/tmp/madara_devnet_poc_v2`)
- `madara-benchmark` repo for generating blocks
- Madara **stopped** before running the POC (RocksDB lock)

### 1) Start Madara and generate blocks
```bash
# Terminal A: start madara devnet
RUST_LOG=info cargo run -p madara --bin madara -- \\
  --name Madara --devnet --rpc --base-path /tmp/madara_devnet_poc_v2 \\
  --chain-config-override=chain_id=POC_DEVNET

# Terminal B: generate blocks with benchmarks
cd /Users/mohit/Desktop/karnot/madara-benchmark/benchmarks
npm run benchmark:oracle
```

Wait until you have the desired block height (example used: ~180).

### 2) Stop Madara
Stop the node (Ctrl+C). This releases RocksDB lock so the POC can open the DB.

### 3) Run POC (squashed mode)
```bash
cargo run --manifest-path madara/node/Cargo.toml --bin madara-trie-poc -- \\
  --db-path /tmp/madara_devnet_poc_v2 \\
  --start-block 0 --block-count 180 --copies 5 \\
  --mode both --parallel-apply squashed \\
  --chain-id POC_DEVNET --allow-non-base \\
  --work-dir /tmp/madara_trie_poc_work --overwrite-work-dir \\
  --output /tmp/madara_trie_poc_results_squashed_180.json
```

Notes:
- If you see `missing state diff for block X`, reduce `--block-count` or choose a different `--start-block`.
- If base DB tip is ahead of `start-block`, use `--allow-non-base`.
- Output JSON now includes `elapsed_us` and `squash_us` per block.

### 4) Run Tests (fixture-based)
```bash
# Fixture-based correctness tests
cargo test -p mc-db test_apply_to_global_trie_from_fixture
cargo test -p mc-db test_parallel_apply_to_global_trie_from_fixture
```

Optional env overrides:
- `MADARA_POC_DB_PATH=/tmp/madara_devnet_poc_v0`
- `MADARA_POC_CHAIN_ID=POC_DEVNET`

## Inputs (Dynamic)
- DB path (Madara devnet).
- Block range length N (CLI/config).
- Copies K (CLI/config).
- Mode: sequential | parallel | both.

## Data Source
- Devnet blocks produced via `madara-benchmark` repo (JS/TS benchmarks).
- State diffs read from Madara DB for blocks x+1..x+N.
- Captured fixture for TDD: `docs/poc_state_updates.json` (blocks 1..10 from `/tmp/madara_devnet_poc_v0`).

## Required DB Data
Minimum needed to run `apply_to_global_trie`:
- 9 trie CFs:
  - `bonsai_contract_{flat,trie,log}`
  - `bonsai_contract_storage_{flat,trie,log}`
  - `bonsai_class_{flat,trie,log}`
- + state CFs (for nonce/class hash lookups):
  - `contract_storage`
  - `contract_nonces`
  - `contract_class_hashes`
- Simplest POC: full DB copy per clone.

## Execution Flow (Baseline Sequential)
1. Determine latest confirmed block `x`.
2. Read state diffs for blocks [x+1..x+N].
3. Clone base DB to a fresh working directory.
4. Run `apply_to_global_trie(start=x+1, diffs)`.
5. Record per-block roots + total elapsed time.

## Execution Flow (Parallel Prefix)
1. Clone base DB K times.
2. For each copy i (1..K): apply diffs [x+1..x+i], record root at x+i.
3. If N > K:
   - Option A: repeat prefix on another batch of copies using the most advanced copy as base.
   - Option B (POC): limit parallel results to first K blocks.
4. Validate: roots must match sequential output for each block.

## Copy State + Rolling Window (Brute Force POC)

### Copy schedule (K=5 example)
- Copy1 handles blocks: x+1, x+6, x+11, ...
- Copy2 handles blocks: x+2, x+7, x+12, ...
- Copy3 handles blocks: x+3, x+8, x+13, ...
- Copy4 handles blocks: x+4, x+9, x+14, ...
- Copy5 handles blocks: x+5, x+10, x+15, ...

### Required diff range for a copy
If copy A currently has state at block X and must compute root for block Y:
- Apply a *squashed diff* over blocks [X+1 .. Y].
- Brute-force POC: recompute squash from JSON-stored diffs each time.

### Diff storage / serialization
- Store **all per-block diffs** in JSON output.
- Measure and record:
  - Serialization time for writing diffs.
  - Deserialization time for reading diffs.
  - Squash time per copy per block.

### Correctness check
- Only validate final roots against sequential.
- Do NOT validate squash separately (root equality implies squash correctness).

## Output (JSON)
Must include:
- run_id / timestamp
- db_path (base + copy paths)
- block_range: start, count
- mode + copies
- per-block roots (sequential + parallel)
- per-block timing (if available)
- total time per mode
- correctness pass/fail (roots match)
- metadata: git commit or build info (optional but helpful)

## Metrics/Logging (for per-block info)
For each block:
- elapsed time for `apply_to_global_trie`
- DB copy creation time
- number of diffs applied per block
- per-copy “work plan” (e.g., copy2 applies blocks x+1..x+2)

Optionally capture:
- total writes to trie columns (if accessible from metrics)
- RocksDB size delta per copy

## CLI / Config Proposal
- `--db-path`
- `--block-count N`
- `--copies K`
- `--mode sequential|parallel|both`
- `--output results.json`
- `--copy-strategy full|trie+state` (default: full)
- `--start-block <optional>`

## Correctness Rules
- For each block b, root_parallel[b] == root_sequential[b]
- Fail fast if any mismatch, include block number + values in JSON.

## Dependencies / Risks
- `apply_to_global_trie` reads state columns (nonce/class hash).
- Pure 9-CF copy is insufficient unless those values are injected.
- Full DB copies are heavier but simplest for POC.

## Implementation Plan (POC Binary)
1. Add new binary crate (e.g., `mc-trie-poc`) in Madara workspace.
2. Implement DB copy utilities (full copy first).
3. Implement sequential + parallel runners with timing.
4. Implement JSON output writer.
5. Provide run instructions (Madara devnet + benchmark + POC).

## Benchmarking Plan (Exact Steps)

### 1. Start from sequential DB
1. Start Madara devnet (tries enabled).
2. Run `madara-benchmark` to generate contracts + txs + blocks.
3. Confirm DB path.
4. Record the block range to use for TDD (default: 1..10).

### 2. Create copy of DB
- Run POC binary in sequential mode first:
  - It will create a full DB copy for isolation.
  - It will read diffs from block range [x+1..x+N].
  - It will measure and store sequential timings and roots.

### 3. Use JS benchmark to create blocks
- Run the JS benchmarks to generate contracts + transactions.
- Required outputs:
  - RPC reachable
  - DB updated with block_state_diff entries

### 4. Store required information across blocks
- In the POC binary:
  - Read diffs from DB for the chosen range.
  - Serialize diffs to JSON.
  - Record serialization + deserialization time.
  - Record per-block root + per-block time.
 - For TDD:
   - Load `docs/poc_state_updates.json` for golden state diffs and expected roots.
   - Do not rely on live RPC during tests.

### 5. Madara changes to capture required info
Minimal changes required:
1) Ensure POC binary can open the DB path reliably.
2) Add tracing or counters for trie updates (optional but helpful).
3) (Optional) Export a “block range snapshot” for debugging.

## Readiness Check
Once POC binary produces JSON:
- Provide the reference data.
- Validate roots match sequential vs parallel.
- Verify JSON contains all necessary fields.
- Check squash and serialization timings.

## TDD Plan (Correctness First)
Goal: ensure the parallel merklelization produces the same roots as the sequential path for a fixed fixture.

### Fixture
- File: `docs/poc_state_updates.json`
- Expected: contains blocks 1..10 with `state_diff` + `new_root`.
- Base DB: `/tmp/madara_devnet_poc_v0` (must match fixture).

### Test Phases
1. **Fixture loader test**
   - Parse JSON and confirm:
     - `block_range` is 1..10.
     - 10 entries exist with `state_diff` and `new_root`.
2. **Sequential correctness test**
   - Clone DB once.
   - Apply diffs 1..10 in order.
   - Assert per-block computed root == fixture `new_root`.
3. **Parallel correctness test**
   - Create K copies (K configurable; default 5).
   - For each copy, squash diffs for its target block and compute root.
   - Assert per-block root == fixture `new_root`.
4. **Regression guard**
   - Fail fast on any mismatch and include block numbers + values in output.

### Notes
- TDD uses fixture JSON only; no RPC dependency.
- Performance measurements are optional for the first test.

## Shadow Code (POC Binary)
```rust
fn main() {
  let cfg = parse_args();
  let base_db = open_backend(cfg.db_path);

  let start_block = cfg.start_block.unwrap_or(base_db.latest_confirmed_block());
  let diffs = load_state_diffs(base_db, start_block+1 .. start_block+N);
  let diff_json = serialize_diffs_to_json(diffs);

  if cfg.mode == "sequential" || cfg.mode == "both" {
     let seq_db = clone_db(cfg.copy_strategy, cfg.db_path);
     let (roots, timing) = run_sequential(seq_db, start_block, diffs);
  }

  if cfg.mode == "parallel" || cfg.mode == "both" {
     let copies = make_copies(cfg.copy_strategy, cfg.db_path, K);
     let plan = build_copy_plan(start_block, N, K);
     let (roots, timing) = run_parallel(copies, plan, diff_json);
  }

  compare_roots(seq_roots, par_roots);
  write_results_json(output_path, all_metadata);
}
```

## Shadow Code (Squash, Brute Force)
```rust
fn squash_diffs(diffs: &[StateDiff]) -> StateDiff {
  let mut merged = StateDiff::default();
  for d in diffs {
    merged.merge_in_place(d);
  }
  merged.normalize();
  merged
}
```

## Future Optimization Hooks (Not Implemented Yet)
- Replace brute-force squash with incremental window updates.
- Use trie-only copy + explicit state injection.
- Parallelize per-copy `apply_to_global_trie` calls.
