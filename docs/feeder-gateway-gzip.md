# Feeder gateway gzip

Madara full nodes advertise `Accept-Encoding: gzip` on feeder GET requests and transparently decode gzip before parsing successful or error JSON responses. The feeder server only compresses successful, non-empty `feeder_gateway/*` responses when the caller accepts gzip.

Server compression is off by default. Enable it with either:

```text
--feeder-gateway-gzip-responses
MADARA_FEEDER_GATEWAY_GZIP_RESPONSES=true
```

Set `MADARA_FEEDER_GATEWAY_GZIP_RESPONSES=false`, or remove the CLI flag, to roll back without changing the binary. Clients require no rollback because identity responses remain supported.

Compression uses the fast gzip level on blocking workers. A process-wide semaphore permits at most four simultaneous compression jobs. Saturated requests and internal compression failures fail open to the original identity response.

The fullnode client caps both the received HTTP body and its decompressed representation at 64 MiB. Invalid, truncated, or oversized responses use the existing bounded five-attempt retry policy, so transient feeder failures can recover without allowing unbounded memory growth.

The `gateway_calls` event keeps the existing route, status, response size, and total duration fields and adds `encoding`, `uncompressed_bytes`, `transmitted_bytes`, and `compression_duration`. Route values are normalized to a fixed set. Operators can calculate the wire-byte saving over a period as:

```text
1 - sum(transmitted_bytes) / sum(uncompressed_bytes)
```

## Rollout

1. Deploy the combined image to Devnet full nodes while sequencer compression remains disabled.
2. Confirm full nodes remain synced, then deploy the image to the Devnet sequencer with compression disabled.
3. Enable feeder gzip on Devnet and observe wire bytes, sequencer CPU, feeder latency/errors, block production, and fullnode/Pathfinder lag.
4. Keep the setting enabled for the agreed observation window and exercise catch-up traffic.
5. Deploy gzip-capable clients to Mainnet before deploying the sequencer image.
6. Enable the Mainnet sequencer setting only after the client rollout is complete.
7. Disable the setting immediately if CPU, errors, timeouts, or sync lag regress.

## Local Devnet validation (2026-09-07)

The validation used an Apple arm64 workstation, a release Madara binary, four concurrent callers, and `eqlabs/pathfinder:v0.22.2` for Linux arm64. The Pathfinder image digest was `sha256:4a117dcb4345fd599eb84c73c560456e4f6f52556e012b78971b5b6ea6bf42b6`.

Compatibility results:

- The new fullnode client synced to height 43 from the preserved pre-change feeder binary. That feeder returned a 766-byte identity response when sent `Accept-Encoding: gzip`.
- The focused two-node integration test passed with the new server both disabled and enabled.
- Pathfinder caught up through the gzip-enabled feeder and remained at zero sampled lag at heights 100 and 166. Its only warning came from the intentionally empty local Anvil chain having no L1 core contract; feeder sync continued normally.
- Restarting the same Madara binary with `MADARA_FEEDER_GATEWAY_GZIP_RESPONSES=false` changed a requested gzip block response back to its 56,261-byte identity representation with no `Content-Encoding` header.

Wire checks used one 100-call Devnet transfer transaction to make block and preconfirmed payloads representative. Each gzip body was a valid stream and normalized to equivalent JSON after decompression. The server unit test additionally checks exact original bytes from a single response:

- `get_block` block 12: 56,261 bytes identity, 2,223 bytes gzip, 96.0% reduction.
- `get_preconfirmed_block` with one 100-call transaction: 56,433 bytes identity, 2,190 bytes gzip, 96.1% reduction.
- `get_state_update` block 12: 1,726 bytes identity, 678 bytes gzip, 60.7% reduction.

The sustained comparison made 3,000 requests for the 56,261-byte block with concurrency four while producing a block every two seconds and keeping Pathfinder attached:

- Identity: 168,783,000 transmitted bytes, 0.750 ms mean latency, 0.955 ms p95, 9.638 seconds wall time, and 1.23 server CPU-seconds.
- Gzip: 6,669,000 transmitted bytes, 0.839 ms mean latency, 1.085 ms p95, 9.919 seconds wall time, and 1.47 server CPU-seconds.
- The load saved 162,114,000 bytes (96.0%) with zero HTTP errors in either run. Average server CPU use increased from about 12.8% to 14.8% of one core, while mean latency increased by 0.089 ms.
- Blocks continued on the configured two-second cadence; sampled block-production work remained 1.6-3.3 ms and Pathfinder finished at height 166 with lag zero.
- Three concurrent Pathfinder/load requests reached the compression bound and safely fell back to identity. Every request in the controlled gzip sample itself returned the expected 2,223-byte gzip representation.

These measurements validate the implementation and local Devnet behavior. The rollout still requires the production-like PC Devnet observation window before Mainnet enablement.
