# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Madara is a hybrid RPC node / sequencer for the Starknet network. It supports both
state synchronization (full node mode) and state production (sequencer mode). The node
follows the Starknet RPC specs and Starknet P2P specs.

**Key capabilities:**

- Full node: Syncs from gateway/sequencer, serves RPC API
- Sequencer: Produces blocks, manages mempool, settles to L1
- Cairo Native execution: AOT compilation support with VM fallback

## Common Commands

### Building

```bash
# Standard build
cargo build --release

# Production build (optimized)
cargo build --profile production

# Development build
cargo build
```

### Running the Node

```bash
# Run with specific network preset
cargo run --release -- --network mainnet
cargo run --release -- --network sepolia

# Run with custom chain config
cargo run --release -- --chain-config-path ./path/to/config.yaml

# Enable Cairo Native execution (AOT compilation)
cargo run --release -- --enable-native-execution

# Sequencer mode
cargo run --release -- --sequencer-mode
```

### Testing

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p mc-db
cargo test -p mc-sync

# Run specific test
cargo test --test test_name

# Run tests with logging
RUST_LOG=debug cargo test
```

### Linting & Formatting

```bash
# Run clippy
cargo clippy --all-targets --all-features

# Format code (max width: 120)
cargo fmt

# Check formatting
cargo fmt -- --check
```

## Architecture Overview

### Crate Organization

**`mc-*` (Client crates)**: Core business logic - how the node transforms data

- `mc-db`: RocksDB storage with Bonsai trie for Merkle state roots
- `mc-sync`: L2 sync service (4-stage pipeline: fetch, resolve, verify, apply)
- `mc-rpc`: JSON-RPC server (v0.7.1, v0.8.1, v0.9.0)
- `mc-exec`: Transaction execution (blockifier integration)
- `mc-block-production`: Block production (batching, aggregation, pending)
- `mc-mempool`: Transaction pool with dynamic scoring
- `mc-settlement-client`: L1 settlement monitoring
- `mc-gateway-server`: Feeder gateway for syncing nodes
- `mc-gateway-client`: Gateway client abstraction
- `mc-class-exec`: Cairo Native execution support
- `mc-telemetry`: Observability and metrics
- `mc-analytics`: Metrics collection
- `mc-devnet`: Development network support
- `mc-submit-tx`: Transaction submission routing

**`mp-*` (Primitives crates)**: Data models and serialization - what data the node transforms

- `mp-block`, `mp-transactions`, `mp-class`, `mp-state-update`, `mp-receipt`
- `mp-chain-config`: Network presets and configuration
- `mp-gateway`, `mp-rpc`: API type definitions
- `mp-convert`: Type conversions
- `mp-utils`, `mp-bloom-filter`: Utilities

**`node/`**: Entry point that wires all services together via `ServiceMonitor`

### Service Orchestration

Services implement the `Service` trait with `ServiceContext` for cancellation.
The `ServiceMonitor` manages lifecycle: registration, activation, graceful shutdown.
Services are identified by atomic bitmask (`PowerOfTwo`) for thread-safe coordination.

Key services run concurrently:

- **L2 Sync**: 4-stage pipeline with parallel processing, sequential state commits
- **L1 Settlement**: 3 workers (state updates, messaging, gas price monitoring)
- **Block Production**: 3-phase execution (batching, aggregation, pending)
- **Mempool**: 4 coordinated queues with dynamic scoring
- **RPC**: User port (9944) + Admin port (9943, localhost-only)
- **Gateway Server**: Feeder gateway endpoints (sequencer-only)
- **Telemetry**: WebSocket broadcast with verbosity levels

### Database Architecture (`mc-db`)

**Backend**: RocksDB with custom column configuration
**State tries**: Bonsai trie for global Merkle state root
**Database migrations**: Version-based with automatic migration runner

**Chain Tip Management:**

- `ChainTip` enum: Empty | Confirmed(block_n) | Preconfirmed(Arc<PreconfirmedBlock>)
- Preconfirmed blocks saved to DB (optional) for crash recovery
- Atomic transitions validated by `MadaraBackendWriter`

**High-Level vs Low-Level API:**

- High-level: `add_full_block_with_classes` - handles everything automatically
- Low-level: Separate functions for headers, txs, state diffs, events, classes
  - Enables partial block storage for sync performance
  - Requires manual head status tracking via `BlockNStatus`
  - Must call `apply_to_global_trie` sequentially
  - Must call `on_full_block_imported` to seal blocks

**Block Views:**

- `DbBlockId` - public API, only sees sealed blocks
- `RawDbBlockId` - internal API, can see partial blocks
- Block views: `MadaraBlockView`, `MadaraConfirmedBlockView`, `MadaraPreconfirmedBlockView`

**Critical**: WAL disabled - manual flush required before shutdown. Periodic flush every N blocks (configurable).

### Execution Model (`mc-exec`)

**Execution Contexts:**

- Block Production: `LayeredStateAdapter` with in-memory cache for uncommitted blocks
- Transaction Validation: Validates against pending state
- RPC Execution: Two modes - block start (tracing) or block end (fee estimation)

**State Adapters:**

- `BlockifierStateAdapter`: Read-only view at specific block
- `LayeredStateAdapter`: Extends with cache queue of recent state changes

**Transaction Flow:**

1. Context initialization (ExecutionContext)
2. Convert API tx → blockifier format
3. Initialize cached state
4. Execute through blockifier
5. Extract execution info, state diff, resource usage

**Re-execution**: Supports block replay for tracing (traceTransaction, traceBlockTransactions)

**State Diff Normalization:** Change detection, deduplication, classification (deployments vs replacements vs updates)

### Cairo Native Execution (`mc-class-exec`)

**Opt-in only**: Enable with `--enable-native-execution` CLI flag
**Dual execution**: Cairo Native (AOT compiled) or Cairo VM fallback
**Multi-tier caching**: In-memory + disk-based compiled classes
**Configuration**: Compilation semaphore, blocking vs async modes, retry logic, per-contract opt-out

### RPC Server Architecture

**Framework**: jsonrpsee (HTTP + WebSocket on same port)
**Middleware**: CORS via tower-http
**Versions**: v0.7.1, v0.8.1, v0.9.0 via path routing (`/rpc/v.../`)
**Structure**: Separate trait implementations per version

- Read methods, Write methods, Trace methods, WebSocket subscriptions
  **Admin endpoints**: Separate port with special methods (ping, shutdown, service control)

### Gateway Architecture

**Client** (`mc-gateway-client`): Fetches from external gateways (Starknet, custom sequencers)
**Server** (`mc-gateway-server`): Serves data to syncing nodes
**Provider pattern**: Gateway + Feeder Gateway URLs
**Disabled by default**: Sequencer-only feature

## Common Patterns

### Error Handling

- **thiserror** for typed errors with context
- **anyhow** for generic errors with context chains
- Errors are contextual: `TxExecError`, `TxFeeEstimationError`, etc. contain view info
- Gateway errors include Starknet-specific error codes

### Configuration

- **Figment**: Multi-source config (CLI, TOML, JSON, YAML, env)
- **Chain config**: Versioned (v1 current), loaded from presets or files
- **Runtime execution config**: Saved with preconfirmed blocks for deterministic replay
- **Network presets**: mainnet, sepolia, devnet

### Concurrency Patterns

- **Rayon**: CPU-bound work (execution, state trie updates)
- **Tokio**: Async I/O (RPC, gateway sync, L1 sync)
- **Channels**: mpsc for pipelines, broadcast for pubsub, watch for shared state
- **Arc + RwLock/Mutex**: Shared mutable state
- **DashMap**: Concurrent hash maps

### Service Communication

- **Direct DB access**: Most services read/write shared database
- **Watch channels**: L1 head updates, chain tip updates
- **Broadcast channels**: Transaction notifications, service status
- **Message queues**: Block production batches, L1→L2 messages

### Testing Patterns

- `#[cfg(test)]` with tempfile for DB
- `rstest` fixtures for setup
- `mockall` for mocking
- `httpmock` for gateway mocking
- Snapshot testing for genesis blocks

## Important Implementation Notes

### When Working with Database Code

- Always use high-level API (`add_full_block_with_classes`) unless you have specific performance needs
- If using low-level API, remember to:
  1. Track head status with `BlockNStatus`
  2. Call `apply_to_global_trie` sequentially
  3. Call `on_full_block_imported` to seal blocks
- Remember WAL is disabled - ensure proper flush before shutdown
- Use `DbBlockId` for public APIs, `RawDbBlockId` only internally

### When Working with Execution Code

- Choose correct state adapter: `BlockifierStateAdapter` for read-only, `LayeredStateAdapter` for block production
- State diff normalization is critical - always deduplicate and classify changes
- Re-execution must maintain determinism - use saved runtime config
- Cairo Native is opt-in only - always provide VM fallback

### When Working with Services

- Implement the `Service` trait with proper `ServiceContext` handling
- Use `PowerOfTwo` for service identification in bitmasks
- Ensure graceful shutdown handling (SIGINT/SIGTERM)
- Use appropriate concurrency primitive: rayon for CPU, tokio for I/O

### When Adding RPC Methods

- Determine which RPC version(s) require the new method
- Implement in appropriate trait (Read/Write/Trace)
- Consider admin vs user endpoint placement
- Add WebSocket subscription if real-time updates needed
- Follow Starknet RPC specs strictly

### When Modifying Block Production

- Preconfirmed block persistence is configurable - check runtime config
- Batcher consumes 3 streams: L1 messages, mempool txs, bypass txs
- Executor runs in separate rayon thread
- Ensure proper block finalization on shutdown
- `LayeredStateAdapter` caching must be consistent

## Code Style

### Linting Rules

- `unsafe_code = "forbid"` - no unsafe code allowed
- `print_stdout/print_stderr = "warn"` - use tracing instead
- `allow-print-in-tests = true` - prints OK in tests
- `allow-unwrap-in-tests = true` - unwrap OK in tests
- `type-complexity-threshold = 250` - higher threshold for AWS SDK errors

### Formatting

- Edition: 2021
- Max width: 120
- Use field init shorthand
- Unix newlines
- Use try shorthand

## Build Profiles

- **dev**: Incremental compilation enabled
- **release**: Panic unwind enabled
- **profiling**: Inherits release + debug symbols
- **production**: LTO, opt-level 3, symbols stripped, single codegen unit
