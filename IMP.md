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
