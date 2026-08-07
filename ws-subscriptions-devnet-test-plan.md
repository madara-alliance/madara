# Websocket Devnet Subscription Test Plan

## Summary

Validate the PR 1012 websocket implementation against a real Madara devnet Docker image, not only unit tests.
The target image for the current head is expected to be:

```text
ghcr.io/madara-alliance/madara:manual-f02ede1
```

Run Madara in devnet mode with a 10-second block time, submit live STRK transfers between two predeployed
accounts, and verify websocket behavior for all supported v0.10.x subscription methods.

## Devnet Harness

- Start Madara with:

```bash
docker run -d --platform linux/amd64 \
  --name madara-ws-subscription-test \
  -p 9944:9944 \
  ghcr.io/madara-alliance/madara:manual-f02ede1 \
  --devnet --preset devnet --rpc --rpc-port 9944 \
  --chain-config-override block_time=10s
```

- Use HTTP URL `http://127.0.0.1:9944/rpc/v0_10_2` and WS URL `ws://127.0.0.1:9944/rpc/v0_10_2`.
- Smoke-test v0.10.0 with `/rpc/v0_10_0`.
- Confirm v0.8.1 and v0.9.0 subscription methods return method-not-found.
- Use two predeployed accounts from the container startup logs.
- Use STRK token transfers as the transaction source.
- For tx-producing tests, submit 4 txs per block: 2 from account A to B and 2 from account B to A.
- Run 10 blocks for focused checks and 20 blocks for the final full pass.
- Capture websocket frames, tx hashes, block tx counts, and Docker logs filtered for `ERROR`, `WARN`, `Missed`, `reorg`, and `subscribe`.

## Endpoint Scenarios

### `starknet_subscribeNewTransactionReceipts`

- Subscribe `PRE_CONFIRMED` filtered by sender A; expect exactly A's 2 receipts per block.
- Subscribe `PRE_CONFIRMED` without sender filter; expect all 4 receipts per block.
- Subscribe `ACCEPTED_ON_L2` filtered by sender A; expect A's 2 receipts at block close.
- Subscribe both `PRE_CONFIRMED` and `ACCEPTED_ON_L2`; expect each matching tx once per finality, with no duplicate refresh sends.
- Repeat the core sender-filter checks on `/rpc/v0_10_0`.

### `starknet_subscribeNewTransactions`

- Subscribe separately to `RECEIVED`, `PRE_CONFIRMED`, and `ACCEPTED_ON_L2` with sender A filter.
- For `PRE_CONFIRMED`, verify appended txs emit before block close.
- For `ACCEPTED_ON_L2`, verify matching txs emit at block close.
- On v0.10.2, verify `INCLUDE_PROOF_FACTS` preserves the normal transaction fields and includes the proof-facts field shape.

### `starknet_subscribeEvents`

- Subscribe to STRK token transfer events using the STRK contract address filter.
- Verify all 4 transfers per block produce matching transfer events when no sender account filter applies.
- Add a transfer selector key filter after reading the emitted event shape, and verify only transfer events are returned.
- Test `PRE_CONFIRMED` and `ACCEPTED_ON_L2` finality separately.
- Verify an empty address array behaves as match-all.

### `starknet_subscribeNewHeads`

- Subscribe from latest; expect one header per closed block with monotonic block numbers.
- Subscribe from a future block number; expect the stream to wait and start at that block.
- Keep this subscription active alongside transaction-heavy blocks to verify headers still arrive while other streams are busy.

### `starknet_subscribeTransactionStatus`

- Subscribe to a tx hash before submitting that tx when possible; verify the observed status progression.
- Subscribe to a nonexistent tx hash and confirm the subscription stays open without spurious frames for at least 2 block intervals.
- Verify reorg notifications still use `starknet_subscriptionReorg` if a reorg is explicitly triggered in a separate local/unit harness.

### `starknet_unsubscribe`

- For each subscription family, receive one valid frame, unsubscribe, and verify no further frames arrive for 2 block intervals.
- Verify invalid string subscription ids return the expected RPC error.
- Verify unsubscribing one client does not stop other active subscribers.

## Cross-Cutting Stress Checks

- Run many-client checks for `N = 5, 50, 100`.
- Reserve `N = 500` for `subscribeNewHeads` unless the machine remains stable.
- For each N:
  - create N clients
  - subscribe all clients
  - produce 2 blocks
  - verify at least one expected frame per active client
  - unsubscribe half
  - verify remaining clients continue and unsubscribed clients stop
- Run a slow-reader check where one client stays connected without reading frames for several blocks while other clients
  continue receiving frames.

## Acceptance Criteria

- No normal devnet scenario emits `Missed reorg notifications`.
- Sender filters return only matching account transactions.
- Preconfirmed streams emit when txs are appended to the current preconfirmed block.
- Confirmed streams emit when blocks close.
- No duplicate notifications for the same `(tx_hash, finality)` pair.
- Unsubscribe stops frames for that subscription and does not break other subscriptions.
- v0.8.1 and v0.9.0 WS subscription methods return method-not-found.
- v0.10.0 and v0.10.2 expose the expected WS subscription methods.
- The final 20-block run passes for receipts, transactions, events, and heads on v0.10.2.
