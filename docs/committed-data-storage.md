# Committed-data storage and replication

Committed data is a generic immutable map of canonical felt keys to felt values. Applications choose their key encoding (for example, an encoded symbol or its hash). Madara does not interpret symbols, prices, expiry or market ownership. The existing special-address call takes `(root, key)` and returns the value; the application still obtains its authorized root from on-chain storage.

## Patricia commitment

The dataset builder sorts keys and rejects duplicates. Empty datasets are rejected; zero is a valid stored value, distinct from a missing key. Leaves commit `PoseidonHashMany([CD_MPT_LEAF_V1, key, value])`. A branch commits `PoseidonHashMany([CD_MPT_BRANCH_V1 + bit, left, right])`, with the domain tags encoded as ASCII felts. Bit zero is the least significant bit. Only differing bits create branches; unary paths are implicit. A dataset has exactly N leaves and N−1 branches.

Witnesses contain `root`, `key`, `value`, and bottom-up `siblings: [{bit, hash}, ...]`. Bits must strictly increase and be below 252. Both Rust execution and the Cairo OS authenticate the same key/value commitment. Maximum path length is 252; typical depth depends on key distribution, not just entry count. Missing data fails closed. This is a binary Patricia construction, not Ethereum's RLP/Keccak trie format.

## Durable records

Imports recompute the root before writing. Entries, branches and metadata are written in one RocksDB batch with a synchronous write-ahead log. Acknowledgement means local durability. Re-import repairs authenticated pages and preserves the first-import checkpoint.

New versions share immutable Patricia nodes in the metadata column family. A root points to an append-only node arena; unchanged subtrees keep their IDs across ticks. An update visits the union of changed paths and publishes new nodes, allocator and root metadata in a single WAL-synced batch. IDs are internal storage offsets, never commitment or client protocol fields.

- Node key: `cdm/n/` followed by an 8-byte big-endian ID; allocator: `cdm/next`.
- Leaf value: tag 0, key (32 bytes), value (32 bytes), totaling 65 bytes. Its hash is recomputed when loaded.
- Branch value: tag 1, critical bit (1), left/right IDs (8 each), hash (32), entry count (4), totaling 54 bytes. Children precede their parent.
- Root metadata: entry count (4), checkpoint-present flag (1), confirmed-head checkpoint (8), root node ID (8), totaling 21 bytes.

Legacy paged Patricia roots remain readable: `committed_data_mpt/ | root (32 bytes) | kind (1 byte) | page (4-byte big-endian)`. Kind 0 holds up to 256 key/value pairs; kind 1 holds postorder branches; kind 255/page 0 has 13-byte metadata. Legacy roots are converted lazily during import/update; the migration does not rewrite chain data.

A cold witness read follows only the requested branch path, reads sibling hashes, and authenticates the resulting witness. Reads use one DB snapshot. Missing roots/keys return no witness; corrupt or incomplete records return an error. Hot execution reads return the value and charged path length without constructing a witness.

The first-import checkpoint records the local confirmed head at admission. It is not a publication or last-use block and cannot alone determine safe pruning. There is no total storage quota or pruning. Future pruning needs root publication/use tracking and a policy covering historical reads and proving. Historical roots share unchanged nodes; changed nodes remain retained. This implementation does not include the separate compact price-delta journal or six-hour/two-day rolling-proof prototype.

## Ingestion and full nodes

Private RPC imports take `entries: [[key, value], ...]`, bounded by the pinned Blockifier `MAX_COMMITTED_DATA_VALUES` limit. The signed import preimage still binds chain ID, root and count. The combined endpoint durably imports all supplied datasets before submitting the signed account transaction through ordinary admission. A multicall is one account transaction. Rejection/reversion can leave unused datasets; this is ordered durable admission, not an atomic DB/chain rollback.

The feeder gateway lists 64 roots per page, serves 4,096 sorted entries per page, and returns witnesses by `root` and felt `key`. Full nodes fetch all entries, recompute the root, then persist locally; they can serve witnesses after source loss. Lexical root cursors require rescanning. The shared additional-data sync option remains unchanged.

The dedicated listener default body limit is 128 MiB to accommodate full-width keys and values; the feeder response limit is 1 MiB and SNOS witness response limit is 32 KiB. These are transport/allocation bounds, not total DB quotas.

This replaces the unreleased indexed-tree format. Existing indexed datasets and roots must be regenerated; there is no legacy reader. SNOS, Blockifier, the OS program hash, Madara and client dataset generation must be upgraded together. No new application-specific configuration is introduced.

## Local durability and asynchronous Mongo recovery

The private import listener requires a sequencer with mempool saving and a configured external DB exporter. Invalid combinations fail before the DB is opened. Mongo can be temporarily unreachable: normal local admission continues until the bounded upload backlog fills. Full-node replication and public committed-data reads do not require the private listener or Mongo.

For each authenticated import, Madara stores a content-addressed manifest and zstd-compressed chunks of submitted upserts, linked to the exact namespace/base manifest. An update against an old dataset without a recovery record requires importing a full checkpoint first. The permanent local manifest is small; the redundant local upload payload is temporary. The exporter uploads parent dependencies first, then chunks, then the complete manifest with majority+journal acknowledgement. Only then is the local payload removed. Historical trie nodes remain available locally.

For combined submission, normal transaction validation and mempool prechecks run before a WAL-synced batch stores the transaction, dependency IDs and optional external outbox row. Block production cannot observe it before that batch succeeds. Admission failures do not evict an existing transaction. A successful response means local durable admission, not execution, finality or completion of remote backup. Lost responses must be reconciled against the returned/computed transaction hash before retrying.

The pending dataset payload budget is 512 MiB. It excludes permanent trie nodes, local manifests, transaction outbox, RocksDB WAL and compaction space. Exhaustion rejects new imports until uploads drain. Keep the existing external writer and pending-byte/error metrics monitored; this is backpressure, not a whole-database size cap. Dataset bytes are compressed in Mongo; transaction documents continue to use the existing external DB format and retention policy.

A same-volume restart reloads saved transactions only after their dataset roots are present. For an older chain DB, the operator explicitly selects `--restore-committed-data-records` and/or `--restore-committed-data-transactions` before services start. Restore checks chain ID, manifest/chunk digests, dependency namespace/base root, decoded input bounds and resulting Merkle roots. Transactions are rehashed, then reconciled against restored-chain nonces through normal mempool startup. The timestamp used for local TTL is refreshed; original arrival time remains in Mongo. Missing or corrupt dependencies fail closed. Restoring an older DB requires a complete remote backup or an independent replay source; async backup does not promise zero data loss if the local volume is lost before upload.

Schema 16 introduces shared nodes, permanent recovery manifests and temporary upload payloads in one additive upgrade from schema 15. RocksDB column registration creates the recovery column; no existing chain records need rewriting. Older binaries must not reopen an upgraded DB. Preserve a pre-upgrade snapshot for rollback. Historical dataset values and trie nodes are not automatically pruned.

Earlier unreleased builds used intermediate schemas 16–19. Those development databases are not an upgrade source for this consolidated schema: recreate them from a pre-feature snapshot or resync rather than editing their version file.
