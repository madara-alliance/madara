# Parallel Bonsai Merkleization POC — Learnings

## What We Ran
- Built a baseline DB from devnet + benchmark (source of truth), then verified all roots against RPC.
- Phase 4 validated two-copy parallel squashing with correctness.
- Phase 5 used 5 bonsai-only copies and ran the squashed-range schedule below, verifying end roots.
- Added read-only source DB support to remove the need for multiple source DB copies.

## Phase 5 Schedule (5 Copies)
Each copy runs sequentially but all copies run in parallel.
- Copy 0: `1..1`, `2..6`, `7..11`, `12..16`, `17..21`, `22..26`
- Copy 1: `1..2`, `3..7`, `8..12`, `13..17`, `18..22`, `23..27`
- Copy 2: `1..3`, `4..8`, `9..13`, `14..18`, `19..23`, `24..28`
- Copy 3: `1..4`, `5..9`, `10..14`, `15..19`, `20..24`, `25..29`
- Copy 4: `1..5`, `6..10`, `11..15`, `16..20`, `21..25`, `26..30`

## Correctness
- Every job checks RPC root for the end block; failures abort the run.
- We also cross-checked every end block root against baseline roots from `tmp/phase1_run/roots.jsonl`.
- Results: **0 mismatches** across all phase-5 runs (both with and without read-only source).

## Performance (Phase 5)
### With source DB copies (per worker)
- Wall time: ~25.53s
- Weighted average per-block: ~848 ms
- Per-copy avg per-block: ~787–939 ms

### With shared read-only source DB
- Wall time: ~46.56s
- Weighted average per-block: ~1,566 ms
- Per-copy avg per-block: ~1,453–1,746 ms

**Note**: The read-only run occurred right after a code change, so cargo rebuilds and lock contention inflated timings. For a fair comparison, run the read-only schedule again with warm build artifacts (or use prebuilt binaries / release mode).

## Scalability Learnings
- Copying a 100s-of-GB source DB does not scale.
- Read-only source DB support allows multiple workers to share a single source DB path safely.
- We still need per-copy bonsai-only DBs for writes, but source DB copies are no longer required.

## Implementation Notes
- Added `RocksDBStorage::open_read_only` and `--source-read-only` flag to `parallel-merklelization`.
- For squashed ranges, the tool builds an override map for non-bonsai reads based on end-block state.
- For the single-block non-squash run (`1..1`), we still use the recorded baseline read map.

## Next Improvements
- Run the read-only schedule with warm caches for accurate timings.
- Move this exact setup to the SSH server once local correctness is stable.
- Consider prebuilding binaries and running them directly to avoid cargo lock overhead during parallel tests.

## RPC Read-Cache + RPC State Diff (Phase 6)
### Feature
- Added `--read-map-out` to write a deduped read-map JSONL from a sequential baseline run.
- Added `--rpc-read-fallback` to fetch missing read-map keys via RPC and cache them in memory.
- Added `--state-diff-source rpc` to fetch per-block state diffs via RPC (no source DB needed).
- Added read-only source support earlier; no longer required for this flow.

### Baseline Run (Full DB)
- Ran sequential apply on a full DB copy with `--apply-state-diff-columns`.
- Output:
  - Calls log: `tmp/phase6_baseline/calls.jsonl`
  - Read map: `tmp/phase6_baseline/read_map.jsonl`
  - Roots: `tmp/phase6_baseline/roots.jsonl`

### Bonsai-only Parallel Run (RPC-driven)
- 5-copy schedule, all squashed ranges, using:
  - `--state-diff-source rpc`
  - `--read-map .../read_map.jsonl`
  - `--rpc-read-fallback`
- Correctness: 0 mismatches vs baseline roots.

### RPC Timing (Aggregated)
- Read fallback: 106 calls, ~2,951 ms total (~27.8 ms avg).
- State diff fetch: 140 calls, ~748 ms total (~5.3 ms avg).

### Performance (Phase 6)
- Wall time: ~23.14s
- Weighted avg per-block: ~786 ms

### Notes
- The read-map is keyed by ReadOp (includes block_n), so a single map can serve all copies.
- RPC state diffs work without a source DB but depend on RPC availability and latency.

## SSH Big-DB Sanity + RPC Squash Fix (2026-02-06)
### Environment
- Host: `ubuntu@52.66.196.211`
- Fast volume: `/mnt/benchmark_io1` (1TB io1)
- DBs involved:
  - Full DB: `/mnt/benchmark_io1/madara` (chain tip: `608679`)
  - Full copy: `/mnt/benchmark_io1/baseline_full_db`
  - Bonsai-only copies: `/mnt/benchmark_io1/madara_bonsai_copy3`, `/mnt/benchmark_io1/madara_bonsai_copy4`, `/mnt/benchmark_io1/madara_bonsai_copy5`

### Key Finding: RPC Squashed Diff Was Incorrect
When the full DB chain tip is behind (here `608679`), we must use `--state-diff-source rpc` to apply blocks `608680+`.

Initial 2-block squash on a bonsai-only copy failed correctness:
- Command shape:
  - `parallel-merklelization --squash-range --state-diff-source rpc --rpc-read-fallback --start-block 608680 --end-block 608681`
- Result: end root mismatch vs RPC.

Root cause:
- `compress_state_diff_no_db` was filtering out `storage_entries` with `value == 0`.
- That is not safe without a DB view because a `0` can represent a real deletion (`pre != 0`, `post == 0`).

Fix:
- Keep zero-valued storage updates in `compress_state_diff_no_db`.
- File: `/Users/mohit/Desktop/karnot/madara2/madara/crates/tools/parallel-merklelization/src/main.rs`

After the fix:
- 2-block squashed apply `608680..608681` on `madara_bonsai_copy5` succeeded and the tool reported: “end block root matches RPC”.

### Read-Map Validation (No DB Reads Needed On Bonsai-Only)
1. Brought `baseline_full_db` from `608679 -> 608681` using `--state-diff-source rpc`.
2. Ran sequential baseline `608682..608691` and wrote a deduped read-map:
   - Read-map: `/tmp/read_map_608682_608691.jsonl`
3. Ran the same 10 blocks on `copy4` and `copy5` using only the read-map:
   - Flags: `--read-map ... --require-read-map --state-diff-source rpc` (no RPC read fallback)

Results:
- All runs ended at block `608691` with identical state root:
  - `0x7718340b1260ceffc3559c42a32c621ed39024a6cc412da99923eb567f8b53d`
- Read-map coverage was complete on bonsai-only:
  - `override_hits = 208`, `override_misses = 0`
- Wall clock:
  - `copy4 elapsed=0:03.69`
  - `copy5 elapsed=0:02.32`

### Benchmark: Baseline Sequential vs 2-Copy Parallel (608692..608701)
Artifact dir (SSH): `/mnt/benchmark_io1/bench_poc_608692_608701_1770374113`

Start state:
- Baseline full DB, copy4 bonsai-only, copy5 bonsai-only were all at `x=608691` before starting.

Baseline (full DB) sequential apply `608692..608701`:
- `wall_clock_total_ms=3087`, `rpc_root_check_ms=131`, `total_minus_root_check_ms=2956`
- `merklization_total_ms=876`
- `rpc_state_diff_total_ms=1879` (10 diffs)
- End root matches RPC: `0x1b6fab3172e4c11bdca853f3205968c7ff767453d94e1702a5ee5f4a9573503`
- Read-map output: `read_map_608692_608701.jsonl` (146 entries)

Parallel 2-copy alternating-root schedule (each segment validates end root vs RPC):
- copy4 segments: `608692..608692`, `608693..608694`, `608695..608696`, `608697..608698`, `608699..608700`
- copy5 segments: `608692..608693`, `608694..608695`, `608696..608697`, `608698..608699`, `608700..608701`
- This produces all 10 roots `608692..608701` (copy4 does even blocks, copy5 does odd blocks).

Per-copy totals (sum over segments):
- copy4:
  - `wall_clock_total_ms=6900`, `rpc_root_check_ms=664`, `total_minus_root_check_ms=6236`
  - `merklization_total_ms=2195`, `squash_total_ms=2860`
  - `rpc_state_diff_total_ms=3380` (9 diffs), `rpc_read_fallback_total_ms=4545` (14 calls, can sum > wall clock due to concurrency)
- copy5:
  - `wall_clock_total_ms=6981`, `rpc_root_check_ms=675`, `total_minus_root_check_ms=6306`
  - `merklization_total_ms=2224`, `squash_total_ms=3426`
  - `rpc_state_diff_total_ms=3423` (10 diffs), `rpc_read_fallback_total_ms=1962` (6 calls, can sum > wall clock due to concurrency)

Service wall time estimate (excluding RPC root validation):
- `~max(copy4.total_minus_root_check_ms, copy5.total_minus_root_check_ms)=6306 ms` to produce roots for `608692..608701`.

## SSH Benchmark: 5 Bonsai Copies (608702..608886)
DBs (SSH, io1): `baseline_full_db` (full), `madara_bonsai_copy{4,5,6,7,8}` (bonsai-only).

Schedule (5 copies): same as Phase-5 schedule but anchored at `start_block` (copy i first runs `start..start+i`, then advances in 5-block squashed segments until `end_block`).

Metric used for comparisons:
- Baseline sequential: `wall_clock_total_ms - rpc_root_check_ms`
- Parallel service: `max_over_copies(sum_over_segments(wall_clock_total_ms - rpc_root_check_ms))`
- RPC timing fields are the tool's summed per-request durations and can exceed wall-clock due to concurrency.

### Results
All runs validated end-root vs RPC for every segment (0 mismatches).

`608702..608711` (10 blocks):
- Baseline: `2399 ms` (~240 ms/block)
- Parallel service: `4962 ms` (~496 ms/block)  (parallel wall: `5582 ms`)
- Slowdown: `2.07x`
- RPC read fallback (critical copy): `12 calls`

`608712..608736` (25 blocks):
- Baseline: `4179 ms` (~167 ms/block)
- Parallel service: `6530 ms` (~261 ms/block)  (parallel wall: `7737 ms`)
- Slowdown: `1.56x`
- RPC read fallback: `0 calls` (all copies)

`608737..608786` (50 blocks):
- Baseline: `8045 ms` (~161 ms/block)
- Parallel service: `15081 ms` (~302 ms/block) (parallel wall: `17292 ms`)
- Slowdown: `1.87x`
- RPC read fallback (critical copy): `22 calls`

`608787..608886` (100 blocks):
- Baseline: `21330 ms` (~213 ms/block)
- Parallel service: `41983 ms` (~420 ms/block) (parallel wall: `46287 ms`)
- Slowdown: `1.97x`
- RPC read fallback (critical copy): `119 calls`

## Hard-Data Instrumentation: Per-Apply Stats (2026-02-06)
### Added `applies.jsonl`
Tool: `/Users/mohit/Desktop/karnot/madara2/madara/crates/tools/parallel-merklelization`

Each run now writes:
- `applies.jsonl`: one JSON record per apply (per-block or squashed-range), including:
  - state-diff size stats (touched contracts, storage entries, etc.)
  - merkleization timings (total + breakdown)
  - per-apply read-hook deltas (nonce/class/storage reads and override hits/misses)
- `run.json` includes `applies_path`.

### Key Local Finding: Squashing Reduces Work (Blocks 21..30 on DEVNET)
Setup:
- All DBs started from the same trie state at block `20` with root:
  - `0x1825a080369eb1d6b79db4e9a3ce7933c014f8b052803cf0f95df1db23fefe0`
- Baseline sequential applied `21..30` per-block on a full DB.
- Parallel run used 5 bonsai-only DBs, each applying a *single* squashed range:
  - copy0: `21..26`
  - copy1: `21..27`
  - copy2: `21..28`
  - copy3: `21..29`
  - copy4: `21..30`

Correctness:
- Each parallel copy end-root matched the baseline root for its end block.
- `override_misses=0` on all squashed runs (nonce/class-hash reads served by overrides).

Work/Time Comparison (baseline `21..30` vs squashed `21..30` on copy4):
- Baseline per-block totals (sum over 10 applies):
  - `touched_contracts_sum=32`
  - `storage_entries_sum=92`
  - `read_events_total=64` (32 nonce + 32 class-hash)
  - `merklization_total_ms=867` (per-block sum, equivalent to pure merkle time)
- Squashed range `21..30` (single apply):
  - `touched_contracts=8`
  - `storage_entries=79`
  - `read_events_total=16` (8 nonce + 8 class-hash)
  - `merklization_total_ms=635`

Conclusion:
- On this dataset, squashing reduced repeated contract-leaf work by ~4x and reduced pure merkle time by ~26%.
- This supports the intuition that batching can reduce redundant ancestor hashing.

### Important Follow-Up: Producing *All* Per-Block Roots Requires Multiple Rounds
The single-apply squash above only produces the end-block root (e.g. `21..30 -> root(30)`).
If the service needs roots for every block `21..30`, the parallel schedule must include multiple applies per copy.

Example 5-copy, 2-round schedule that produces *all* roots `21..30`:
- Copy0: `21..21` then `22..26` (roots: 21, 26)
- Copy1: `21..22` then `23..27` (roots: 22, 27)
- Copy2: `21..23` then `24..28` (roots: 23, 28)
- Copy3: `21..24` then `25..29` (roots: 24, 29)
- Copy4: `21..25` then `26..30` (roots: 25, 30)

Local benchmark (fresh copies at state 20), correctness checked against baseline roots:
- Sequential baseline (full DB, per-block `21..30`):
  - `wall_clock_total_ms=1070`, `wall_clock_compute_ms=964`, `merklization_total_ms=935`
- Parallel schedule (5 copies, 2 applies per copy):
  - Overall wall-clock (start -> all applies finished): `2632 ms`
  - Critical-path compute (max over copies of seg1+seg2 compute): `1161 ms`
  - Critical-path pure merkle (max over copies of seg1+seg2 merkle): `1130 ms`

Note:
- The measured overall wall-clock is inflated because we spawn 10 separate processes (2 runs per copy), and each run re-opens DBs.
- A real orchestrator/microservice would keep each copy's DB open and execute multiple applies in-process, pushing wall time closer to the critical-path compute time.

## Important Benchmarking Pitfall: Process Model (Warm vs Cold Caches)
When comparing `per_block` (sequential) vs `squash_range` (parallel schedule), we must ensure the *process model* is comparable:
- The default seq bench runs **one long-lived process** applying many blocks.
- The default par schedule runs **one process per segment** (DB is re-opened per segment).

This can bias `merklization_ms` because RocksDB block cache / memtables and bonsai internal working sets can be warm in the long-lived seq process but cold in the per-segment par processes.

Sanity experiment (SSH):
- Warm seq (single process, 100 blocks): `merklization_ms ~ 0.205*key_total + 1.48`
- Par schedule (one process per segment, 100 applies): `merklization_ms ~ 0.362*key_total + 1.92`
- Cold seq (one process per block, 50 blocks): `merklization_ms ~ 1.01*key_total - 22.8`
- Par schedule on same 50-block window: `merklization_ms ~ 0.96*key_total - 23.7`

Takeaway:
- The large “par slope > seq slope” effect is largely a **warm vs cold cache** artifact of how we ran the benchmark, not necessarily inherent extra trie work.
- For a realistic microservice, each worker should keep its DB handle open and run multiple segments in-process.

### Removing Contention (Same Work, Run Copies in Isolation)
If we run each copy's segments without concurrency (no CPU/I/O contention) and then take the max per-copy merkle time
(this approximates "dedicated resources per copy" or an orchestrator controlling threads), we get:
- Sequential baseline merkle (full DB, per-block `21..30`): `673 ms`
- Parallel critical-path merkle (5 copies, 2-round schedule): `424 ms`

Interpretation:
- The earlier "parallel merkle is slower" result was dominated by oversubscription (multiple multithreaded processes at once).
- Under no contention, the critical-path merkle time is lower than sequential as expected.

## Keys vs Merkleization Time (SSH, Pure Merkle, No RPC Fallback) (2026-02-07)
### Dataset
Block window:
- `608897..608996` (100 blocks)

Environment assumptions:
- All runs used `--require-read-map` with a global read-map (so `rpc_read_fallback_count=0`).
- We did not include RPC validation time in `merklization_ms` (it measures Bonsai commit time only).

Artifacts:
- CSV: `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449/records.csv` (600 rows)
- Plots:
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449/plots/key_total_vs_merkle_ms.png`
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449/plots/storage_keys_vs_storage_commit_ms.png`
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449/plots/touched_contracts_vs_contract_trie_commit_ms.png`
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449/plots/merkle_ms_per_key_by_block_count.png`

How we defined "keys" in this dataset:
- `key_total = touched_contracts + unique_storage_keys + class_updates_total`
- In this window `class_updates_total` was always `0`, so `key_total ~= touched_contracts + unique_storage_keys`.

### Simple Fit (per apply)
Linear regression `merklization_ms ~ key_total`:
- Sequential (per-block applies): `merklization_ms = 2.2593*key_total + 33.07` (`R^2=0.790`, `n=300`)
- Parallel schedule (mostly 5-block squashed applies): `merklization_ms = 2.3501*key_total + 50.42` (`R^2=0.777`, `n=300`)

### Better Model (what actually explains variance)
Multivariate least-squares fit:
- Sequential: `merklization_ms ~= 0.7383*unique_storage_keys + 21.7591*touched_contracts - 21.72` (`R^2=0.969`)
- Parallel: `merklization_ms ~= 0.4665*unique_storage_keys + 27.0630*touched_contracts - 0.91*block_count - 21.66` (`R^2=0.975`)

Interpretation:
- `touched_contracts` has a large fixed cost component (contract-leaf work).
- `unique_storage_keys` drives storage-commit work roughly linearly.
- `block_count` provides some amortization (small in this dataset because it is usually `5`).

### Note: Debug vs Release Builds Matter a Lot
The original `bench_keys_time_...` iters `1..3` were produced with a slower build (effectively debug-like timings).
When we rebuilt and ran later iterations with a `--release` binary (iters `4..8`), absolute timings dropped by ~10x,
but the *shape* of the relationship stayed the same.

Release-only artifacts (iters `4..8`):
- CSV: `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449_release_iters4_8/records.csv`
- Plots:
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449_release_iters4_8/plots/key_total_vs_merkle_ms.png`
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449_release_iters4_8/plots/storage_keys_vs_storage_commit_ms.png`
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449_release_iters4_8/plots/touched_contracts_vs_contract_trie_commit_ms.png`
  - `/Users/mohit/Desktop/karnot/madara2/tmp/bench_keys_time_608897_608996_1770451449_release_iters4_8/plots/merkle_ms_per_key_by_block_count.png`

Release-only fits (per apply):
- Sequential (per-block applies): `merklization_ms = 0.2105*key_total + 1.36` (`R^2=0.602`, `n=500`)
- Parallel schedule (mostly 5-block squashed applies): `merklization_ms = 0.3655*key_total + 1.95` (`R^2=0.760`, `n=500`)

Release-only multivariate fits:
- Sequential: `merklization_ms ~= 0.0268*unique_storage_keys + 2.566*touched_contracts - 5.255` (`R^2=0.831`)
- Parallel: `merklization_ms ~= 0.0693*unique_storage_keys + 4.249*touched_contracts - 0.0306*block_count - 9.912` (`R^2=0.959`)

### Practical Single-Block Estimator (Warm Worker)
Assumptions:
- **Release build**, and a **long-lived worker** (DB handle stays open; RocksDB caches/memtables are warm).
- **Single-block apply** (`block_count=1`).
- Read-map is complete (`--require-read-map`) so there is no RPC read-fallback on the critical path.
- RPC validation time is excluded (we are modeling `merklization_ms` only).

Derived from: `/Users/mohit/Desktop/karnot/madara2/tmp/bench_bonsai_ops_608897_608996_1770464272/records.csv` (`kind=seq`, 10 repeats per block).

Estimator:
- `merklization_ms ~= max(0, -3.0 + 0.057*unique_storage_keys + 1.92*touched_contracts + 12.9*deployed_contracts)`
- Rough mental model: `~ max(0, -3 + 0.06*unique_storage_keys + 2*touched_contracts + 13*deployed_contracts)` (ms).

How to compute inputs from a state diff:
- `unique_storage_keys`: number of distinct `(contract_address, storage_key)` updates in `storage_diffs`.
- `touched_contracts`: number of distinct contracts touched by storage updates, nonce updates, deployments, or class replacements.
- `deployed_contracts`: `len(deployed_contracts)`.

Sanity checks (measured vs predicted):
- Block `608950` had `(unique_storage_keys=30, touched_contracts=5, deployed_contracts=0)`, predicted `~8.33 ms`, measured `8–9 ms`.
- Block `608910` had `(unique_storage_keys=52, touched_contracts=10, deployed_contracts=1)`, predicted `~32.10 ms`, measured `31–33 ms`.

Pitfall:
- Comparing `per_block` vs `squash_range` with `block_count=1` is not evidence about squashing/parallel behavior.
  Both modes apply a single block's diff, so they are expected to match; this only sanity-checks instrumentation.
