# Committed-data storage and replication

Committed data is a generic immutable map of canonical felt keys to felt values. Applications choose their key encoding (for example, an encoded symbol or its hash). Madara does not interpret symbols, prices, expiry or market ownership. The existing special-address call takes `(root, key)` and returns the value; the application still obtains its authorized root from on-chain storage.

## Patricia commitment

The dataset builder sorts keys and rejects duplicates. Empty datasets are rejected; zero is a valid stored value, distinct from a missing key. Leaves commit `PoseidonHashMany([CD_MPT_LEAF_V1, key, value])`. A branch commits `PoseidonHashMany([CD_MPT_BRANCH_V1 + bit, left, right])`, with the domain tags encoded as ASCII felts. Bit zero is the least significant bit. Only differing bits create branches; unary paths are implicit. A dataset has exactly N leaves and N−1 branches.

Witnesses contain `root`, `key`, `value`, and bottom-up `siblings: [{bit, hash}, ...]`. Bits must strictly increase and be below 252. Both Rust execution and the Cairo OS authenticate the same key/value commitment. Maximum path length is 252; typical depth depends on key distribution, not just entry count. Missing data fails closed. This is a binary Patricia construction, not Ethereum's RLP/Keccak trie format.

## Durable records

Imports recompute the root before writing. Tree nodes and metadata are written in one RocksDB batch with a synchronous write-ahead log. Acknowledgement means local durability. Re-import repairs authenticated tree nodes and preserves the first-import checkpoint.

New versions share immutable Patricia nodes in the metadata column family. A root points to an arena with monotonically allocated IDs; unchanged subtrees keep their IDs across ticks. An update visits the union of changed paths and publishes new nodes, allocator and root metadata in a single WAL-synced batch. IDs are internal storage offsets, never commitment or client protocol fields.

- Node key: `cdm/n/` followed by an 8-byte big-endian ID; allocator: `cdm/next`.
- Leaf value: tag 0, key (32 bytes), value (32 bytes), totaling 65 bytes. Its hash is recomputed when loaded.
- Branch value: tag 1, critical bit (1), left/right IDs (8 each), hash (32), entry count (4), totaling 54 bytes. Children precede their parent.
- Root metadata: entry count (4), checkpoint-present flag (1), confirmed-head checkpoint (8), root node ID (8), totaling 21 bytes. ID zero means the proof index has been reclaimed; the values remain archived.

Root metadata uses `committed_data_mpt/ | root (32 bytes) | 255 | 0 (4-byte big-endian)`. Only the shared-node format is supported; unreleased paged snapshots must be recreated.

A retained-index witness read follows only the requested branch path, reads sibling hashes, and authenticates the resulting witness. Reads use one DB snapshot. Missing roots/keys return no witness; corrupt or incomplete records return an error. Hot execution reads return the value and charged path length without constructing a witness.

## Settlement-gated proof retention

Successful durable ingestion wakes one background proof-cleanup worker. Private imports notify it after the local recovery journal is durable; full-node replication notifies it after persisting a dataset. Triggers during an active sweep are discarded, not queued. There is no periodic cleanup timer or additional cadence setting. Reads do not trigger cleanup. Without new ingestion, cleanup stays idle even if L1 settlement advances; another successful ingestion starts the next eligible sweep. The worker checks shutdown between roots and awaits each admitted DB mutation. Capacity planning must include settlement lag, time until another ingestion, and sweep duration.

Historical values never expire. Enable `--committed-data-proof-retention-secs 172800` for a two-day buffer behind the latest L1-confirmed **L2** height. Omission retains every proof index. The cutoff is:

```text
cutoff = min(L1-confirmed L2 height, local confirmed height)
       - ceil(buffer seconds / configured block time)
```

Only roots last imported/used **strictly before** the cutoff are eligible. No settlement or insufficient history means no pruning. Stalled settlement extends the retained window; this is not a wall-clock TTL. Imports conservatively mark the next block after the observed head; account execution records its actual execution height. Historical execution below the cutoff does not prolong retention. Reusing a pruned root in recent execution restores its proof index once.

The first-import checkpoint remains unchanged and is not used as a last-use marker. Shared nodes carry reference counts for branch edges and retained roots. Releasing one root removes only nodes with no remaining references, in the same WAL-synced batch that marks that root cold. Readers hold RocksDB snapshots, so pruning cannot tear an in-flight read.

The `cdh/v1/` archive stores one content-addressed, sorted key dictionary per key set. Checkpoints contain compact values; subsequent records contain dictionary-index gaps and signed price deltas. Values outside the small-delta range use canonical absolute felts, including zero and negative field representations. All records are zstd-compressed. Every root has an explicit parent and at most 64 deltas before a full value checkpoint. Adding keys starts a new dictionary/checkpoint; forks with the same key set reuse the dictionary. This changes storage only: Merkle roots, leaf hashes, and witnesses retain their protocol format. Mongo export records remain independent.

This is the feature's first storage format. There is no reader or migration for earlier PR1261 drafts. Each archive and dictionary is authenticated before releasing proof nodes. Missing or corrupt history prevents pruning that root. Archive verification runs outside the import write lock, then eligibility is rechecked under that lock. Reclamation evicts only deleted nodes from the bounded cache. Historical values and dictionaries are never deleted; a future value-retention policy must account for dictionary and checkpoint dependencies.

Older proofs remain available without an age cutoff. Madara reconstructs the canonical tree from retained values, verifies its root, then produces the requested proof. A batch reconstructs once for all requested keys. Only one reconstruction runs per backend at a time, and ordinary old-proof requests do not repopulate the durable index. Older paginated value reads also reconstruct; repeated pages or single-key calls can therefore be slow. This is a correctness-first cold path, not a cold-query throughput guarantee.

### Memory and backup controls

An old-proof rebuild is temporary and is dropped before its read returns, including on error. It is not inserted into a reusable tree cache or persisted for a later sweep. One rebuild per backend bounds concurrent reconstruction; repeated old reads may repeat the work. Dropping a tree releases its allocations, but the allocator may retain pages, so process RSS need not fall immediately. A recent execution that deliberately reuses an old root is different: its restored durable index follows the normal last-use retention cutoff.

- `--committed-data-memory-blocks 7200`: cached objects expire after this many observed blocks; zero disables reuse. At a five-second block time, 7200 blocks is ten hours.
- `--committed-data-cache-mib 256`: estimated reusable-cache ceiling, split equally between decoded Patricia nodes and the existing two-full-import cache. Both portions also obey the block window. Zero disables caching. It is **not** a process RSS limit.
- `--committed-data-pending-backup-mib 512`: serialized pending dataset upload budget. Exhaustion applies backpressure; it never discards pending records. This queue is on disk, not a 512 MiB RAM reservation.

Equivalent environment variables are `MADARA_COMMITTED_DATA_PROOF_RETENTION_SECS`, `MADARA_COMMITTED_DATA_MEMORY_BLOCKS`, `MADARA_COMMITTED_DATA_CACHE_MIB`, and `MADARA_COMMITTED_DATA_PENDING_BACKUP_MIB`.

### Sizing and measurement

Permanent values continue growing with updates, even after proof pruning. Approximate live storage as:

```text
permanent checkpoints + permanent upserts + metadata
+ shared proof nodes for (L1 settlement lag + retention buffer)
+ reference edges + pending backups
```

Count new shared nodes, not N full trees per block. One leaf consumes 79 logical bytes including its key; one branch consumes 68. Each reference record adds 40 logical bytes (16-byte key, 24-byte value). Compression, RocksDB indexes, WAL, obsolete SSTs and compaction headroom must be measured separately. Deletion creates tombstones; disk usage need not fall until compaction completes. Mongo history is a separate storage cost.

RAM includes reusable caches **plus** import staging, reference-count batches, a bounded-concurrency cold reconstruction, RocksDB caches/memtables and the rest of Madara. The cache setting alone cannot size a machine. A dataset is still limited to 524,288 entries by the pinned commitment implementation.

The ignored `committed_history_measure_storage_and_memory` check exercises real imports, pruning, compaction and identical retained/reconstructed batch proofs. `CD_MEASURE_ENTRIES`, `CD_MEASURE_UPDATES`, `CD_MEASURE_VERSIONS`, and `CD_MEASURE_RETAINED` select the workload; `CD_MEASURE_REPORT` saves JSON. It reports logical bytes, live SST bytes before/after compaction, Linux RSS/high-water RSS, import timings and cold-proof timings. Keys are deterministic full-width felts. Price mode uses captured price-width deciles with synthetic nonzero changes every tick; large-movement mode independently redraws each price. Generic mode covers full-width values. ACK timings include the backend import and durable recovery journal, but exclude transport, signatures and account transaction validation; exporter ACKs are simulated. A separate rolling phase includes pruning each tick. RSS includes the independent reference-root construction. These are functional measurements, not production capacity or p95 predictions.

The manual **Task - Integration Tests (Madara)** workflow can run that measurement from an exact-revision CI test archive without rebuilding. Supply its successful CI run ID and workload parameters. Normal PR checks exclude this opt-in measurement.

## Ingestion and full nodes

Private RPC imports take `entries: [[key, value], ...]`, bounded by the pinned Blockifier `MAX_COMMITTED_DATA_VALUES` limit. The signed import preimage still binds chain ID, root and count. The combined endpoint durably imports all supplied datasets before submitting the signed account transaction through ordinary admission. A multicall is one account transaction. Rejection/reversion can leave unused datasets; this is ordered durable admission, not an atomic DB/chain rollback.

The feeder gateway lists 64 roots per page, serves 4,096 sorted entries per page, and returns witnesses by `root` and felt `key`. Full nodes fetch all entries, recompute the root, then persist locally; they can serve witnesses after source loss. Lexical root cursors require rescanning. Enable `--madara-additional-data-sync` (or `MADARA_ADDITIONAL_DATA_SYNC=true`) to replicate both bouncer weights and committed datasets. This replaces the previous `--bouncer-config-sync-enable` / `MADARA_ENABLE_BOUNCER_CONFIG_SYNCING` setting; update deployment arguments and environment variables when upgrading.

The dedicated listener default body limit is 128 MiB to accommodate full-width keys and values; the feeder response limit is 1 MiB and SNOS witness response limit is 32 KiB. These are transport/allocation bounds, not total DB quotas.

Databases created with earlier PR1261 drafts must be regenerated; those drafts are not supported upgrade sources. SNOS, Blockifier, the OS program hash, Madara and client dataset generation must be upgraded together. No new application-specific configuration is introduced.

## Local durability and asynchronous Mongo recovery

The private import listener requires a sequencer with mempool saving and a configured external DB exporter. Invalid combinations fail before the DB is opened. Mongo can be temporarily unreachable: normal local admission continues until the bounded upload backlog fills. Full-node replication and public committed-data reads do not require the private listener or Mongo.

For each authenticated import, Madara stores a content-addressed manifest and zstd-compressed chunks of submitted upserts, linked to the exact namespace/base manifest. An update against an old dataset without a recovery record requires importing a full checkpoint first. The permanent local manifest is small; the redundant local upload payload is temporary. The exporter uploads parent dependencies first, then chunks, then the complete manifest with majority+journal acknowledgement. Only then is the local payload removed. Historical values remain available in the independent local archive; retained proof indexes accelerate recent reads.

For combined submission, normal transaction validation and mempool prechecks run before a WAL-synced batch stores the transaction, dependency IDs and optional external outbox row. Block production cannot observe it before that batch succeeds. Admission failures do not evict an existing transaction. A successful response means local durable admission, not execution, finality or completion of remote backup. Lost responses must be reconciled against the returned/computed transaction hash before retrying.

The pending dataset payload budget defaults to 512 MiB and is configurable. It excludes permanent trie nodes, local manifests, transaction outbox, RocksDB WAL and compaction space. Exhaustion rejects new imports until uploads drain. Keep the existing external writer and pending-byte/error metrics monitored; this is backpressure, not a whole-database size cap. Dataset bytes are compressed in Mongo; transaction documents continue to use the existing external DB format and retention policy.

A same-volume restart reloads saved transactions only after their dataset roots are present. For an older chain DB, the operator explicitly selects `--restore-committed-data-records` and/or `--restore-committed-data-transactions` before services start. Restore checks chain ID, manifest/chunk digests, dependency namespace/base root, decoded input bounds and resulting Merkle roots. Transactions are rehashed, then reconciled against restored-chain nonces through normal mempool startup. The timestamp used for local TTL is refreshed; original arrival time remains in Mongo. Missing or corrupt dependencies fail closed. Restoring an older DB requires a complete remote backup or an independent replay source; async backup does not promise zero data loss if the local volume is lost before upload.

Schema 16 introduces shared nodes, permanent recovery manifests and temporary upload payloads in one additive upgrade from schema 15. RocksDB column registration creates the recovery column; no existing chain records need rewriting. Older binaries must not reopen an upgraded DB. Preserve a pre-upgrade snapshot for rollback. Historical values remain permanent; proof pruning is explicitly opt-in. The local history records and reference indexes belong to this same first released feature schema.

Earlier unreleased builds used intermediate schemas 16–19. Those development databases are not an upgrade source for this consolidated schema: recreate them from a pre-feature snapshot or resync rather than editing their version file.
