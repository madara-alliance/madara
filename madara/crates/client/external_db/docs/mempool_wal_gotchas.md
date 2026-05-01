# Mempool WAL replay gotchas

This document summarizes mempool acceptance rules that matter when replaying
transactions from the external DB WAL. The WAL stores every accepted mempool
transaction (including duplicates), but the mempool itself enforces additional
constraints that can cause replays to be rejected if the state has diverged.

## Nonce rules

- Nonce too low is rejected immediately.
  - If `tx.nonce < account_nonce`, the tx is rejected (`NonceTooLow`).
- Future nonce is allowed for most txs and becomes pending.
  - **Declare** is the exception: declare tx must be exactly the current
    account nonce; otherwise it is rejected (`PendingDeclare`).

## Duplicate handling

- Duplicate tx hash is rejected by the mempool (`DuplicateTxn`).
  - The WAL is a log and **must** allow duplicates even though mempool rejects
    them later.
- Same account+nonce is a conflict unless replacement rules are satisfied.
  - In tip-based mode, replacement requires a minimum tip bump.
  - In timestamp/FCFS mode, only older transactions can replace newer ones.

## TTL and aging

- Transactions older than the TTL can be rejected during insertion.
- Reinserted transactions keep their original `arrived_at`, so replaying very
  old entries may fail the TTL check.

## Reorg / preconfirmed behavior

- Account nonces can move backward during reorg/preconfirmed rollback, which
  can change readiness of pending transactions.
- A replay that was valid at original time can be rejected if the nonce/state
  has moved.

## L1 handler

- L1 handler transactions are rejected by the submit validation path and do
  not enter the mempool.

## Practical replay guidance

- Replay in order of `arrived_at` (then a stable UUID tie-breaker).
- Expect and tolerate mempool rejections; WAL is an archive, not a guarantee
  of acceptance on replay.
- Ensure the chain state is reverted/consistent before replay when comparing
  global roots.

## Mempool DB persistence vs external DB WAL

- If mempool saving is enabled, transactions can be reloaded from the local
  mempool DB on restart while also being replayed from the external DB WAL.
  This can cause duplicate submission errors or unexpected ordering.
- For WAL replay flows, prefer disabling mempool saving via
  `--no-mempool-saving` (or `MADARA_NO_MEMPOOL_SAVING=1`) and document the
  choice in your setup to avoid hidden conflicts.
