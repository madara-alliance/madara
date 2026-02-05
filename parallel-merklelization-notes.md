# Parallel Merklelization Notes (Madara / Bonsai)

Date: 2026-02-04
Branch: codex/parallel-merklelization-rework
Scope: Read-only understanding of current Madara structure, Bonsai trie usage, log behavior, and current DB state. No code changes.

## Current DB State (remote)
Host: ubuntu@52.66.196.211
DB path: /madara_db/madara/db

Latest known values from meta:
- Highest state-diff block: 463337
- Highest trie block (LATEST_APPLIED_TRIE_UPDATE): 440469
- Gap: 22868

How it was obtained:
- Meta key LATEST_APPLIED_TRIE_UPDATE decoded as bincode varint u64 prefix 0xFC + u32 LE
- Meta key CHAIN_TIP decoded to Confirmed(463337)
- Verified block_state_diff entry exists for 463337 and not for 463338

## Where State Diffs Are Applied
Files:
- madara/crates/client/sync/src/import.rs
- madara/crates/client/sync/src/apply_state.rs
- madara/crates/client/db/src/rocksdb/global_trie/mod.rs

Key flow:
- Importer reads latest_applied_trie_update and skips already-applied ranges.
- apply_to_global_trie applies state diffs sequentially to Bonsai tries and writes latest_applied_trie_update.
- Snap-sync mode accumulates diffs and applies a single squashed diff, using block number = last block in range for fallback reads.

## Bonsai Trie Integration (Madara)
Files:
- madara/crates/client/db/src/rocksdb/trie.rs
- madara/crates/client/db/src/rocksdb/global_trie/contracts.rs
- madara/crates/client/db/src/rocksdb/global_trie/classes.rs
- madara/crates/client/db/src/rocksdb/global_trie/mod.rs

Highlights:
- 3 Bonsai trees: contract, contract_storage, class
- 9 Bonsai column families are used
- Contract leaf hash uses class_hash, storage_root, nonce, and 0
- Contract leaf computation falls back to DB reads for class_hash and nonce if they are missing from the diff

## RocksDB Columns
File: madara/crates/client/db/src/rocksdb/column.rs

Bonsai-related column families:
- bonsai_contract_flat
- bonsai_contract_trie
- bonsai_contract_log
- bonsai_contract_storage_flat
- bonsai_contract_storage_trie
- bonsai_contract_storage_log
- bonsai_class_flat
- bonsai_class_trie
- bonsai_class_log

## Madara State Diff Compression
File: madara/crates/client/sync/src/sync_utils.rs

compress_state_diff behavior:
- For storage diffs, checks if contract existed at pre_range_block via get_class_hash_at.
- For existing contracts, compares each storage entry against get_storage_at(pre_range_block).
- For new contracts, filters out zero values.
- For replaced classes, compares previous class hash at pre_range_block.
- Uses concurrent tasks, but all checks are local DB reads.

Non-Bonsai reads used by compress_state_diff:
- get_contract_class_hash_at
- get_storage_at

Additional fallback reads during apply:
- get_contract_nonce_at
- get_contract_class_hash_at

## Bonsai Trie Logs and Reorg/Proofs
Source: bonsai-trie (madara-alliance/bonsai-trie) at /Users/mohit/.cargo/git/checkouts/bonsai-trie-86d624db5853ef3f/43829ef
File: src/key_value_db.rs

Key behavior:
- get_transaction reconstructs transactional state by replaying trie logs between snapshot and target id.
- Missing logs result in a transaction error ("database is missing trie logs").
- Setting max_saved_trie_logs = 0 disables this, which means no transactional state, no reorg using logs, no storage proofs.
- For POC, logs can be disabled.

## Orchestrator Squashing Reference
Files:
- orchestrator/src/compression/squash.rs
- orchestrator/src/compression/utils.rs
- orchestrator/src/compression/batch_rpc.rs

Summary:
- Builds a StateDiffMap and merges the latest updates per key across a range.
- Uses batch RPC calls for pre-range class hash and storage checks.
- Filters no-ops similarly to Madara compress_state_diff, but RPC-backed and batched.

## Parallel Merklelization POC Plan (as discussed)
Baseline:
- Use the existing sequential DB as source of truth.
- Apply state diffs sequentially and capture the non-Bonsai reads for each block.

Copies:
- Create multiple Bonsai-only copies (Copy 0..Copy 4) from the baseline at block X.
- Copies apply state diffs in parallel, with squashing where needed.
- Non-Bonsai reads are provided from a prebuilt mapping (JSON) recorded during baseline runs.

Correctness:
- Compare global state roots against baseline after each copy applies its target block(s).
- Global root equality is the correctness signal.

Timing:
- Squashing time counts toward POC timing.
- Squashing can be done in parallel or precomputed before applying in the copies.

Copying strategy note:
- Do not delete databases unless a correctness issue indicates corruption.
- For benchmarking, keep all copies; ensure they are synced to the same state before proceeding.

Clarifications added 2026-02-04:
- "Route" refers to global state root per block (and final root).
- Start block should be latest_applied_trie_update + 1; end block depends on run length.
- Bonsai-only copies may not have META; track latest applied trie update externally (JSON).
- Baseline should validate DB state root against RPC block header before running (to ensure DB is correct).
- Non-Bonsai calls should be captured into a JSON mapping and loaded at runtime by Bonsai-only copies.
- Bonsai-only copies must be strictly the 9 Bonsai CFs (smaller size).
- Correctness gates each phase; proceed only after repeated success.
- Missing JSON mapping entries should hard-fail (no fallback).
- Correctness can be checked at end block; if end root matches RPC header, prior roots are assumed correct.

## Open Questions To Resolve Later
- JSON mapping schema for baseline reads, including block height and call type.
- How to route fallback reads (nonce, class hash, storage) in code without modifying behavior beyond the POC harness.
- How to open RocksDB with only Bonsai CFs while still using Madara code paths (may require a dedicated "bonsai-only" backend).
- RPC validation scope (per-block vs preflight sampling).
- Repeat count for "multiple times" success gate between phases.
