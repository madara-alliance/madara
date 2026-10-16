# Committed-data storage and replication

Committed data is a generic immutable map of canonical felt keys to felt values. Applications choose their key encoding (for example, an encoded symbol or its hash). Madara does not interpret symbols, prices, expiry or market ownership. The existing special-address call takes `(root, key)` and returns the value; the application still obtains its authorized root from on-chain storage.

## Patricia commitment

The dataset builder sorts keys and rejects duplicates. Empty datasets are rejected; zero is a valid stored value, distinct from a missing key. Leaves commit `PoseidonHashMany([CD_MPT_LEAF_V1, key, value])`. A branch commits `PoseidonHashMany([CD_MPT_BRANCH_V1 + bit, left, right])`, with the domain tags encoded as ASCII felts. Bit zero is the least significant bit. Only differing bits create branches; unary paths are implicit. A dataset has exactly N leaves and N−1 branches.

Witnesses contain `root`, `key`, `value`, and bottom-up `siblings: [{bit, hash}, ...]`. Bits must strictly increase and be below 252. Both Rust execution and the Cairo OS authenticate the same key/value commitment. Maximum path length is 252; typical depth depends on key distribution, not just entry count. Missing data fails closed. This is a binary Patricia construction, not Ethereum's RLP/Keccak trie format.

## Durable records

Imports recompute the root before writing. Entries, branches and metadata are written in one RocksDB batch with a synchronous write-ahead log. Acknowledgement means local durability. Re-import repairs authenticated pages and preserves the first-import checkpoint.

Records live in the existing metadata column family. Keys are `committed_data_mpt/ | root (32 bytes) | kind (1 byte) | page (4-byte big-endian)`. Pages contain up to 256 records:

- Kind 0: sorted `(key, value)` pairs, 64 bytes each.
- Kind 1: branches in postorder, each `hash (32) | critical bit (1) | left ID (4) | right ID (4)`, 41 bytes. Leaf IDs are `0..N`; branch IDs follow them. Children always precede their parent.
- Kind 255, page 0: entry count (4), checkpoint-present flag (1), confirmed-head checkpoint (8), totaling 13 bytes.

A cold witness read follows only the requested branch path, reads sibling hashes, and authenticates the resulting witness. Reads use one DB snapshot. Missing roots/keys return no witness; corrupt or incomplete records return an error. Hot execution reads return the value and charged path length without constructing a witness.

The first-import checkpoint records the local confirmed head at admission. It is not a publication or last-use block and cannot alone determine safe pruning. There is no total storage quota or pruning. Future pruning needs root publication/use tracking and a policy covering historical reads and proving. Each root currently stores its own complete dataset; this change does not introduce cross-root node sharing.

## Ingestion and full nodes

Private RPC imports take `entries: [[key, value], ...]`, bounded to 524,288 entries per import. The signed import preimage still binds chain ID, root and count. The combined endpoint durably imports all supplied datasets before submitting the signed account transaction through ordinary admission. A multicall is one account transaction. Rejection/reversion can leave unused datasets; this is ordered durable admission, not an atomic DB/chain rollback.

The feeder gateway lists 64 roots per page, serves 4,096 sorted entries per page, and returns witnesses by `root` and felt `key`. Full nodes fetch all entries, recompute the root, then persist locally; they can serve witnesses after source loss. Lexical root cursors require rescanning. The shared additional-data sync option remains unchanged.

The dedicated listener default body limit is 128 MiB to accommodate full-width keys and values; the feeder response limit is 1 MiB and SNOS witness response limit is 32 KiB. These are transport/allocation bounds, not total DB quotas.

This replaces the unreleased indexed-tree format. Existing indexed datasets and roots must be regenerated; there is no legacy reader. SNOS, Blockifier, the OS program hash, Madara and client dataset generation must be upgraded together. No new application-specific configuration is introduced.
