# Scratchpad: `starknet_getMessagesStatus` (Madara)

This file is the running source of truth for the work to enable `starknet_getMessagesStatus` on current `main`.

## Ground Truth (Current Repo State)

### Where we are (repo + branches)
- Current branch: `codex/fix/get-message-status` (forked from `main` at `cf3f1bc8b`).
- Separate branch for out-of-scope proptest stabilization: `codex/chore/bloom-filter-proptest` (draft PR opened).
- Reference branch (outdated): `origin/feat/get-msg-status` (single commit `073909ad2`).
- Implementation commits on this branch:
  - `40a78e13f`: `mp-rpc` types for `getMessagesStatus` (v0.9.0 + re-export in v0.10.0)
  - `4edb50936`: `mc-db` L1 tx hash -> (nonce -> maybe L2 tx hash) secondary index
  - `adc43fdf6`: `mc-db` nonce -> L1 tx hash index + block-write fill-in
  - `4ef98fa93`: settlement sync persists origin metadata (no RPC-in-RPC)
  - `601d013e5`: `mc-rpc` implements `starknet_getMessagesStatus` (v0.9.0 + v0.10.0)
  - `fc95abc6e`: refactor marker writes (decouple get/insert), remove replay repair-fill logic, fix doctests

### Review feedback applied (2026-02-10)
- README: keep the supported-methods row for `starknet_getMessagesStatus` but removed the redundant per-row version note.
- RPC handler (`mc-rpc`): removed unnecessary comments, added a unit test that covers multiple messages under the same L1 tx hash.
- DB (`mc-db`): clarified public backend APIs with docs and refactored `messages_to_l2_write_trasactions` to call a small helper.
- Settlement sync (`mc-settlement-client`): added an explicit **backfill** step:
  - if `nonce -> l2_tx_hash` is already present when we observe the L1 event, we fill `(l1_tx_hash||nonce) -> l2_tx_hash`.
- Tests:
  - added/expanded test docs (what is being tested and desired result)
  - added an `mc-e2e-tests` end-to-end lifecycle test:
    - marker-only => RPC returns `[]`
    - after writing a consumed L1-handler block => RPC returns `ACCEPTED_ON_L2` + `SUCCEEDED`.

### Build note (new)
- `mp-rpc` derives `serde::{Serialize, Deserialize}` for many types that contain `starknet_types_core::felt::Felt`.
  - This requires enabling the `serde` feature on `starknet-types-core` (workspace had only `hash` enabled).
- `mp-rpc` types also use `Arc<...>` in many places (e.g. signatures/calldata).
  - This requires enabling serde's `rc` feature (for `Arc` impls).

### Local test running notes (important)
- We should not skip/ignore tests in-tree for this feature.
- `mc-e2e-tests` (used by `mc-settlement-client` Starknet integration tests) requires:
  - `COVERAGE_BIN` env var pointing to the `madara` binary (see `scripts/e2e-tests.sh`).
  - `RUST_LOG=info` (otherwise the Madara process may not emit the `Running JSON-RPC server at ...` line that the
    harness uses to detect ports, causing 30s startup timeouts).
- The ETH anvil-fork based tests require an **archive-capable** `ETH_FORK_URL`.
  - `https://api.zan.top/eth-mainnet` is usable but can hit rate limits under the full test suite.
  - `https://ethereum.publicnode.com` worked reliably for the full `mc-settlement-client` test suite locally.
  - Running `mc-settlement-client` tests reliably also requires `-- --test-threads=1` (avoids `#[traced_test]` global subscriber races).

### Collaboration note (Mohit preference)
- When explaining new concepts: build gut-level intuition with concrete examples, give alternative perspectives,
  and include short exercises/questions to check understanding (not passive reading).

### What `origin/feat/get-msg-status` actually changed (file list)
Commit `073909ad2` touched (high signal subset):
- RPC:
  - `madara/crates/client/rpc/src/versions/user/v0_8_1/api.rs` (added the method to the RPC trait for v0.8.1 in that older layout)
  - `madara/crates/client/rpc/src/versions/user/v0_8_1/methods/read/get_messages_status.rs` (new handler)
  - `madara/crates/client/rpc/src/versions/user/v0_8_1/methods/read/mod.rs` (wired the handler)
  - `madara/crates/client/rpc/src/lib.rs` (added `settlement_client: Option<Arc<dyn SettlementLayerProvider>>` onto `Starknet`)
- Node wiring:
  - `madara/node/src/service/rpc/mod.rs` (plumbed `settlement_client` into `Starknet::new(...)`)
  - `madara/node/src/service/l1.rs` (provided the settlement client to RPC service)
- Settlement client:
  - `madara/crates/client/settlement_client/src/client.rs` (added `SettlementLayerProvider::get_messages_to_l2(l1_tx_hash)`)
  - `madara/crates/client/settlement_client/src/eth/mod.rs` and `.../starknet/mod.rs` (implemented the new method per provider)
- RPC primitives:
  - `madara/crates/primitives/rpc/src/custom/l1.rs` + `.../custom/mod.rs` (added custom serde + types like `L1TxnHash`)
  - `madara/crates/primitives/rpc/src/v0_8_1/mod.rs` and `.../v0_8_1/starknet_api_openrpc.rs` (re-exports / glue)

### Current status on `main`
- On `main` (at `cf3f1bc8b`), `starknet_getMessagesStatus` was not implemented.
- On branch `codex/fix/get-message-status`, `starknet_getMessagesStatus` is implemented for:
  - RPC `v0.9.0`
  - RPC `v0.10.0`
  - Intentionally not implemented for RPC `v0.8.1` (per Mohit requirement).
- README indicates it is not yet implemented:
  - `README.md` shows `starknet_getMessageStatus` as 🚧 (note: the name is singular there, which likely does not match the official spec).

### Why the old branch is stale (high level)
The work on `origin/feat/get-msg-status` assumed:
- A different RPC version module layout (it edited/added `.../v0_8_1/api.rs`, which does not exist on current `main`).
- `mc_rpc::Starknet` had access to a settlement client via `starknet.settlement_client()`.
  - On current `main`, `mc_rpc::Starknet` does **not** contain any settlement/L1 client handle (it only has backend, ws handles, etc.).
- The response shape it used matches Starknet spec `v0.8.x` only (missing `execution_status` which is required by spec `v0.9.0+` / `v0.10.0`).

## Official Starknet Spec (What the RPC Must Match)

Source used: `starkware-libs/starknet-specs` (OpenRPC).

### Method name
- Official method name: `starknet_getMessagesStatus` (plural: **Messages**).

### Parameter
- Single required param:
  - `transaction_hash` (schema: `L1_TXN_HASH`)
  - Description: "The hash of the L1 transaction that sent L1->L2 messages"
  - Note: `L1_TXN_HASH` is `NUM_AS_HEX` in the spec (`^0x[a-fA-F0-9]+$`). We accept up to 64 hex digits and left-pad.

### Semantics (answer to “L1->L2 vs L2->L1?”)
- This method is specifically for **L1 -> L2** messaging:
  - input is an **L1** transaction hash (the tx that emitted one or more `LogMessageToL2` events)
  - output is the list of corresponding **L2** L1-handler transaction hashes + their statuses (once known)
- It is **not** about L2 -> L1 messages (those are exposed via receipts / `messages_sent` etc, not via this endpoint).

### Result (version differences)

Spec `v0.8.1`:
- Result is an array of objects:
  - `transaction_hash` (`TXN_HASH`) [required]
  - `finality_status` (`TXN_STATUS`) [required]
  - `failure_reason` (string) [optional; only if `finality_status` is `REJECTED`]

Spec `v0.9.0` and `v0.10.0`:
- Result is an array of objects:
  - `transaction_hash` (`TXN_HASH`) [required]
  - `finality_status` (`TXN_FINALITY_STATUS`) [required]
  - `execution_status` (`TXN_EXECUTION_STATUS`) [required]
  - `failure_reason` (string) [optional; only if `execution_status` is `REVERTED`]

Implication:
- There is **no** way to represent “RECEIVED” / “message seen but not yet executed on L2” in this response for v0.9+
  because `finality_status` is `TXN_FINALITY_STATUS` (pre_confirmed / accepted_on_l2 / accepted_on_l1 only).
- Therefore, for v0.9.0/v0.10.0 we must either:
  - omit entries until we have a real L2 tx receipt/status (recommended), or
  - violate the spec (not acceptable).

Decision (Mohit, aligned with spec):
- If DB has **no** keys for `l1_tx_hash` => return `TXN_HASH_NOT_FOUND`.
- If DB has keys but values are not filled yet => return `[]` (this is the spec-compliant equivalent of “received/seen”).
- For cancelled/invalid messages: still record the `(l1_tx_hash, nonce)` presence marker so the L1 tx hash is “known”.
  - We cannot expose a “cancelled” state via this endpoint; absence of consumed tx hashes yields `[]`.

Important nuance:
- `finality_status` here is **TXN_FINALITY_STATUS**, not TXN_STATUS.
  - TXN_FINALITY_STATUS enum is only: `PRE_CONFIRMED`, `ACCEPTED_ON_L2`, `ACCEPTED_ON_L1`.
  - So we **cannot** return `RECEIVED` for “seen but not yet consumed” messages while staying spec-compliant.
- Because `execution_status` is required, we also cannot return entries for transactions we cannot produce a receipt for.
  - Practically: return only consumed L1-handler txs (those that exist on L2), and omit “not yet consumed”.

### Errors
- Includes `TXN_HASH_NOT_FOUND` (for unknown L1 tx hash).

## What `origin/feat/get-msg-status` Implemented (for reference)

### RPC side
It added a `v0_8_1` read method `get_messages_status` roughly like:
1. Accept `L1TxnHash` (custom type).
2. Use `starknet.settlement_client().get_messages_to_l2(l1_tx_hash)` to fetch L1->L2 messages.
3. For each message:
   - Compute the L1 handler tx hash (`msg.tx.compute_hash(...)`).
   - Call `get_transaction_status(...)` to get finality status.
   - Return `Vec<TxnHashWithStatus { transaction_hash, finality_status, failure_reason: None }]`.

Key limitations vs latest spec:
- No `execution_status` in the response (required in `v0.9.0+` / `v0.10.0`).
- It hardcodes `failure_reason: None` due to missing support.

### Types side
It introduced custom RPC types like:
- `L1TxnHash` (a 32-byte hash with `0x` hex serde)
- `TxnHashWithStatus` (hash + status + optional reason)

None of these exist on current `main`.

## Current Architecture Constraints (What blocks implementation on `main`)

### RPC server (`mc_rpc`)
- `mc_rpc::Starknet` currently has no access to a settlement-layer provider/client.
- Node startup (`madara/node/src/service/rpc/mod.rs`) constructs `Starknet::new(...)` with:
  - backend, submit tx provider, storage proof config, block production handle, service context
  - no L1 client/provider injected

### Settlement client (`mc_settlement_client`)
- The settlement client is structured as:
  - `L1ClientImpl` (outer coordinator, owns DB + provider)
  - `SettlementLayerProvider` (inner ETH/Starknet-specific implementation)
- It supports streaming messages (`messages_to_l2_stream`) and consuming pending messages for block production, but:
  - There is **no** existing API to query "messages sent by a specific L1 transaction hash".
- The streaming metadata contains `l1_transaction_hash`, but DB persistence is keyed by nonce and by nonce->L1Handler txn hash; it does not index by L1 tx hash.

## Likely Work Needed (Design Direction, not implemented yet)

This section was the initial “direction”. It is superseded by the updated DB-backed plan below, but the high-level tasks remain:

This is now implemented on `codex/fix/get-message-status`:
1. DB indexing during settlement sync and block write path, so the RPC handler is pure DB.
2. `mp_rpc` types (`L1TxnHash`, `MessageStatus`).
3. `getMessagesStatus` implemented only for RPC `v0.9.0` and `v0.10.0` (ignore `v0.8.1`).
4. README naming mismatch fixed (plural).

## Notes / Pitfalls

- There is already an internal `mp_convert::L1TransactionHash([u8; 32])` on `main`, but its current serde derives likely do not match the RPC spec (OpenRPC expects a `0x...` hex string for `L1_TXN_HASH`, not a JSON array of bytes). Any RPC-facing type should implement hex-string serde.

## Pathfinder Reference (What Another Node Does)

Pathfinder implements `starknet_getMessagesStatus` by:
1. Fetching the L1 transaction receipt for the given `L1TransactionHash`.
2. Filtering `LogMessageToL2` logs from the Starknet core contract in that receipt.
3. Converting each log into an L1 handler tx (and then hashing it to get the L2 tx hash).
4. Calling `getTransactionStatus` for each derived L2 tx hash and returning the list.

Notable behavior:
- It conditionally includes `execution_status` only for RPC `v0.9+` and skips entries without it.
- I did not find dedicated tests for this method in pathfinder, but the implementation is clear and matches the spec intent.

### Pathfinder L1_TXN_HASH Parsing (important)
In `/Users/mohit/Desktop/karnot/og-pathfinder`:
- The input param `transaction_hash` for `starknet_getMessagesStatus` is deserialized as `H256` and then wrapped into
  `pathfinder_common::L1TransactionHash(H256)` (see `crates/rpc/src/method/get_messages_status.rs`).
- Their `H256` parser is custom (`crates/rpc/src/dto/primitives.rs`): it uses `hex_str::bytes_from_hex_str_stripped::<32>`.
  Behavior:
  - accepts optional `0x` prefix (so `"abc"` works),
  - accepts 0..=64 hex digits (so `"0x"` parses as 0),
  - accepts odd nibble length and left-pads to 32 bytes.

Comparison to our current `mp_rpc::v0_9_0::L1TxnHash`:
- We also accept 1..=64 hex digits, odd nibble lengths, and left-pad to 32 bytes.
- We are stricter than pathfinder by requiring a `0x` prefix and at least one digit after it (spec-compliant).

## Implementation Plan (TDD-Oriented)

### Architecture (updated: DB-backed, no RPC-in-RPC)
Goal: `starknet_getMessagesStatus` should not make any additional settlement-layer requests at RPC time.

Data flow:
1. The L1→L2 messaging sync worker (already running in `mc_settlement_client`) consumes message events. Each event already contains:
   - `l1_transaction_hash` (the originating tx hash on the settlement layer)
   - the reconstructed `L1HandlerTransactionWithFee` (includes `nonce`, calldata, etc.)
2. While syncing, we persist minimal “origin” metadata in RocksDB:
   - `nonce -> l1_tx_hash` (point lookup; lets us associate later when the L1 handler tx is actually included).
   - `l1_tx_hash + nonce -> <empty>` (prefix index + presence marker; preserves ordering by nonce without needing the L2 tx hash yet).
3. When an L1 handler tx is actually included in our chain (block write path), we *fill in* the index value:
   - `l1_tx_hash + nonce -> l1_handler_tx_hash` (from the actual receipt / stored tx hash), so it is correct w.r.t. protocol version.
4. JSON-RPC `starknet_getMessagesStatus(l1_tx_hash)` becomes a **pure DB query**:
   - iterate over all `(l1_tx_hash + nonce -> maybe_tx_hash)` entries for that `l1_tx_hash` (ordered by nonce)
   - for entries which have a tx hash value:
     - derive status using existing local `getTransactionStatus` and return it
   - for entries with an empty value:
     - omit (v0.9+ requires `execution_status`)

Main tradeoff / limitation:
- The node can only answer for L1 tx hashes it has observed via the L1 messaging sync worker (from its replay window / saved tip onward). Older L1 tx hashes will return `TXN_HASH_NOT_FOUND` unless the node replays older events.

### Scope Constraints (per Mohit, 2026-02-07)
- Ignore RPC version `v0_8_1`.
- Implement only for RPC `v0_9_0` and `v0_10_0` (confirmed by Mohit).

### DB Design Discussion (nonce record vs multiple columns)
Mohit idea: since nonce is the “source of truth” for messages, store a single nonce-keyed record that contains
all relevant fields (l1_tx_hash, l2 tx hash, message data, fee, etc) to avoid proliferating columns.

Key constraint for `getMessagesStatus`:
- RPC input is `l1_tx_hash`, so we need an efficient `l1_tx_hash -> {messages}` lookup.
- A *pure* `nonce -> record` store would require scanning all nonces at request time, which is not acceptable.

Conclusion:
- We still need a **secondary index** keyed by `l1_tx_hash` (at least).
- We can potentially reduce new columns by embedding `l1_tx_hash` inside the pending-message value (requires
  backward-compatible decoding) and only adding the `l1_tx_hash||nonce -> maybe l2_tx_hash` index.

### Related Work (Prakhar PR #967)
PR: `madara-alliance/madara#967` (branch `fix/message-syncing-tip-wen-reverting`, commit `b872d17e...`).
Key changes:
- Adds RocksDB column `l1_to_l2_paid_fee_by_nonce` to persist `paid_fee_on_l1` for confirmed L1->L2 messages.
- Updates revert logic to restore L1->L2 messages as pending on chain reverts, using stored fee if available.
- Touches `madara/crates/client/db/src/rocksdb/l1_to_l2_messages.rs` (same area we’d likely extend for our new index).

Impact on our plan:
- No overlap with the `l1_tx_hash -> message` indexing we need for `getMessagesStatus`.
- But it means we should minimize merge conflicts by putting our new index in a separate DB module/file where possible, and just register a new column in `ALL_COLUMNS`.

### Step-by-step (planned)
1. `mp_rpc` types (DONE: `40a78e13f`)
   - Add `L1TxnHash` (0x-hex string serde, 32 bytes).
   - Add per-version response item types:
     - `mp_rpc::v0_9_0::MessageStatus { transaction_hash, finality_status, execution_status, failure_reason? }`
     - `mp_rpc::v0_10_0::MessageStatus { ...same as v0_9_0... }` (re-export)

2. DB index (mc_db) (DONE: `4edb50936`)
   - Added RocksDB column:
     - `l1_to_l2_l2_txn_hash_by_l1_txn_hash_and_nonce`:
       - Key: `<l1_tx_hash 32 bytes><nonce u64 big-endian>` (prefix extractor len = 32)
       - Value: either empty (seen on L1 but not yet consumed) OR `<l2_tx_hash 32 bytes>` (the L2 `l1_handler` tx hash) once included on L2.
   - Added storage APIs:
     - `ensure_message_to_l2_seen_on_l1(l1_tx_hash, nonce)` (writes empty marker without clobbering filled values)
     - `write_message_to_l2_consumed_txn_hash(l1_tx_hash, nonce, l2_tx_hash)`
     - `get_messages_to_l2_by_l1_tx_hash(l1_tx_hash) -> Option<Vec<(nonce, Option<l2_tx_hash>)>>`

3. Store L1 tx hash by nonce (mc_db) (DONE: `adc43fdf6`)
   - Added RocksDB column:
     - `l1_to_l2_l1_txn_hash_by_nonce`:
       - Key: `<nonce u64 big-endian>`
       - Value: `<l1_tx_hash 32 bytes>`
   - Integrated into block write path:
     - when a consumed L1-handler tx is written and we know `nonce`, look up `l1_tx_hash` by nonce and fill the `(l1_tx_hash||nonce) -> l2_tx_hash` index value.

4. Write “origin” metadata during message syncing (mc_settlement_client) (DONE: `4ef98fa93`)
   - In `process_finalized_events` (where we pop finalized message events):
     - write `nonce -> l1_tx_hash`
     - write placeholder entry `l1_tx_hash + nonce -> empty`
   - Do this even if the message is later skipped for cancellation; it was still emitted by that L1 tx.
   - Repair-fill behavior:
     - if we already have the consumed `nonce -> l2_tx_hash` mapping (message was consumed before we saw the L1 tx hash), we immediately fill `(l1_tx_hash||nonce) -> l2_tx_hash`.

5. RPC method implementation (mc_rpc) (DONE: `601d013e5`)
   - Add `getMessagesStatus` to the versioned read RPC traits:
     - `v0_9_0`, `v0_10_0`
   - Implement handlers that:
     - iterate the DB index for the provided `L1TxnHash`
     - if there are zero keys: return `TxnHashNotFound`
     - if keys exist but values are empty: return `[]` (spec-compliant; cannot return “RECEIVED” in v0.9+)
     - for keys with tx hash values: fetch the local **transaction receipt** and map it to:
       - `finality_status` (from receipt common properties)
       - `execution_status` + `failure_reason` (from receipt execution status / revert reason)
     - Rationale: this avoids any “status shape” mismatch (spec wants `TXN_FINALITY_STATUS` + `TXN_EXECUTION_STATUS`)
       and gives us the revert reason without extra APIs.
   - v0.10.0 can delegate to v0.9.0 implementation (same response shape).

6. Docs (IN PROGRESS)
   - Fix README method name to `starknet_getMessagesStatus` (plural) and update status.
   - Add explicit note: implemented only for RPC `v0.9.0` and `v0.10.0`; intentionally not for `v0.8.1`.

### Test Coverage (current)
Unit tests first; no network.

1. `mp_rpc` serde tests (DONE)
   - `L1TxnHash` serialize/deserialize:
     - canonical 0x + 64 hex digits
     - reject missing `0x` prefix (spec-compliant; stricter than pathfinder)
     - reject invalid hex / >32 bytes

2. DB index tests (mc_db) (DONE)
   - `nonce -> l1_tx_hash` roundtrip.
   - placeholder write + later fill-in write, then iterate by prefix and assert ordering.
   - Revert-like reset (set value to empty) keeps key present.

3. Messaging sync tests (mc_settlement_client) (DONE)
   - Extend existing `messaging_module_tests` to assert that processing a finalized message event writes:
     - `nonce -> l1_tx_hash`
     - `l1_tx_hash+nonce -> empty`
   - Write the placeholder index even when the message is later deemed invalid/cancelled, so `getMessagesStatus(l1_tx_hash)`
     returns `[]` (not `TXN_HASH_NOT_FOUND`) for L1 tx hashes we have actually observed.
   - No external network: use `MockSettlementLayerProvider` as existing tests do.

4. RPC handler tests (DONE)
   - Seed the DB index for a chosen `l1_tx_hash`:
     - case A: placeholder only (no filled values) => returns `[]`
     - case B: filled values + chain contains tx receipts => returns statuses (+ failure reason when reverted)
   - Seed chain state so some `l1_handler_tx_hash` exists in a stored block (so we can read a stable receipt).
   - Assertions:
     - v0_9_0 / v0_10_0 include only tx hashes with execution status available (skip pending/unseen)
     - empty index => `TxnHashNotFound`

### Settlement Provider (Eth vs Starknet) Notes
- Madara can be configured to listen for L1->L2 messaging events from either:
  - Ethereum (L1=Ethereum), or
  - Starknet (L1=Starknet, i.e. L3-style setups).
- The `getMessagesStatus` storage layout is provider-agnostic because it stores the *raw 32-byte tx hash*:
  - ETH provider: `eth/event.rs` maps `log.transaction_hash (B256)` into `U256`, then we store `U256::to_be_bytes::<32>()`.
  - Starknet provider: `starknet/event.rs` maps `event.transaction_hash (Felt)` into `U256` via `to_u256()`, then we store the same 32-byte big-endian form.
- Test situation:
  - We have unit tests for ETH and Starknet event conversion (each asserts `l1_transaction_hash` mapping).
  - We have unit tests for DB indices and RPC behavior.
  - We now have e2e tests in `mc-settlement-client` which assert, end-to-end, that the new DB indices are written for:
    - ETH provider (`eth/mod.rs::e2e_test_basic_workflow`)
    - Starknet provider (`starknet/mod.rs::e2e_test_basic_workflow_starknet`)

### L1TxnHash (Deserializer) Confidence
- Added an RPC-level e2e test (in `mc-e2e-tests`) verifying that `starknet_getMessagesStatus`:
  - accepts short/odd hex like `0x1` and `0xabc` (left-padded),
  - rejects invalid formats like missing `0x` or `0x` with no digits (invalid params).

### 2026-02-13: DB Version + CI Follow-up
- **DB versioning gap identified:** this PR adds new RocksDB columns/indexes, so DB schema version must be bumped and migration registry updated.
- **Applied local fix (not pushed):**
  - `.db-versions.yml`: `current_version` bumped `9 -> 10`, added v10 history entry.
  - Added no-op migration: `madara/crates/client/db/src/migration/revisions/revision_0010.rs`
  - Exported revision in `migration/revisions/mod.rs`
  - Registered `v9 -> v10` in `migration/registry.rs`
- **Why no-op migration is enough:** new column families are created lazily/open-time, so no key rewrite is required.
- **mempool-wal interaction:** PR `#960` also uses `v10`. When rebasing onto/after `mempool-wal`, this branch will likely need migration chain conflict resolution (expected next step).
- **CI root cause on PR #971:** `lint-code-style / prettier` failed only on `README.md` formatting.
- **Local CI fix applied (not pushed):** ran `prettier --write README.md` and validated in a clean detached worktree that `npx prettier --check .` passes.

### 2026-02-13 (update): version offset from mempool-wal
- Confirmed from PR `#960` that mempool-wal bumps DB to **v10**.
- Adjusted this branch to **v11** (`mempool-wal + 1`) so the intended chain is:
  - `v9 -> v10`: mempool/external DB column addition (no-op)
  - `v10 -> v11`: getMessagesStatus L1-to-L2 index addition (no-op)
- Updated files:
  - `.db-versions.yml` to `current_version: 11`, with v11 and v10 history entries.
  - `migration/revisions/mod.rs` to export `revision_0011`.
  - Added `migration/revisions/revision_0011.rs`.
  - `migration/registry.rs` now registers `8->9`, `9->10`, and `10->11`.
- Validation: `cargo test -p mc-db migration::registry::tests::test_` passes.
