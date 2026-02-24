<!-- markdownlint-disable -->
<div align="center">
  <img src="https://github.com/keep-starknet-strange/madara-branding/blob/main/logo/PNGs/Madara%20logomark%20-%20Red%20-%20Duotone.png?raw=true" width="500" alt="Madara logo">
</div>

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/madara-alliance/madara)
[![Workflow - Push](https://github.com/madara-alliance/madara/actions/workflows/release-publish.yml/badge.svg)](https://github.com/madara-alliance/madara/actions/workflows/push.yml)
[![Project license](https://img.shields.io/github/license/madara-alliance/madara.svg?style=flat-square)](LICENSE)
[![Pull Requests welcome](https://img.shields.io/badge/PRs-welcome-ff69b4.svg?style=flat-square)](https://github.com/madara-alliance/madara/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
[![Follow on Twitter](https://img.shields.io/twitter/follow/madara-alliance?style=social)](https://twitter.com/madara-alliance)
[![GitHub Stars](https://img.shields.io/github/stars/madara-alliance/madara?style=social)](https://github.com/madara-alliance/madara)

# 🥷 Madara: Starknet Client

Madara is a high-performance Starknet client written in Rust, supporting both full-node and sequencer operation.

## Highlights (Current State)

- User JSON-RPC versions: `v0.7.1`, `v0.8.1`, `v0.9.0`, `v0.10.0` (default: `v0.10.0`)
- Admin JSON-RPC version: `v0.1.0` (optional, disabled by default)
- SnapSync for faster state synchronization (`--snap-sync`)
- Cairo Native execution (opt-in, VM fallback remains available)
- L3-friendly operation with Starknet settlement (`--settlement-layer starknet`)
- Automatic DB migration framework with resumable upgrades
- Warp-update sender/receiver flow for fast local database migration
- External MongoDB-backed mempool outbox support (optional)

## Table of Contents

- [Installation](#installation)
  - [Option A: Install Binary (`madaraup`)](#option-a-install-binary-madaraup)
  - [Option B: Build from Source](#option-b-build-from-source)
  - [Run Modes](#run-modes)
- [Run with Docker](#run-with-docker)
- [Configuration](#configuration)
  - [Configuration Precedence](#configuration-precedence)
  - [CLI Presets](#cli-presets)
  - [Useful Runtime Flags](#useful-runtime-flags)
- [RPC and Interactions](#rpc-and-interactions)
  - [User RPC Endpoints](#user-rpc-endpoints)
  - [Admin RPC Endpoints](#admin-rpc-endpoints)
  - [Supported Method Families](#supported-method-families)
  - [Discover Methods Dynamically](#discover-methods-dynamically)
- [Sync and Migration Strategy](#sync-and-migration-strategy)
  - [Sync Modes](#sync-modes)
  - [Database Migration Lifecycle](#database-migration-lifecycle)
  - [Warp Update Workflow](#warp-update-workflow)
- [Repository Components](#repository-components)
- [Get in Touch](#get-in-touch)

## Installation

> [!TIP]
> For an easier local setup, see [Dev Containers](.devcontainer/README.md).

### Option A: Install Binary (`madaraup`)

```bash
curl -L https://install.madara.build | bash
madaraup --help
```

### Option B: Build from Source

1. Clone:

```bash
git clone https://github.com/madara-alliance/madara.git
cd madara
```

2. Install dependencies:

| Dependency | Version     | Installation                                                      |
| ---------- | ----------- | ----------------------------------------------------------------- |
| Rust       | `1.89`      | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Clang      | recent      | `sudo apt-get install clang`                                      |
| OpenSSL    | `0.10` libs | `sudo apt-get install openssl libssl-dev`                        |

> [!TIP]
> If you enable Cairo Native, install LLVM 19 (`make install-llvm19`) first.

3. Build the node workspace:

```bash
cargo build --manifest-path madara/Cargo.toml --release
```

### Run Modes

> [!NOTE]
> Node commands below are executed from the repository root and target `madara/Cargo.toml`.

#### Full Node (Mainnet)

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --name madara \
  --full \
  --base-path /var/lib/madara \
  --network mainnet \
  --l1-endpoint "$ETHEREUM_RPC_URL"
```

#### Sequencer

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --name madara \
  --sequencer \
  --base-path /var/lib/madara \
  --preset sepolia \
  --l1-endpoint "$ETHEREUM_RPC_URL"
```

#### Devnet

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --name madara \
  --devnet \
  --base-path /tmp/madara-devnet \
  --chain-config-override chain_id=MY_CUSTOM_DEVNET
```

> [!CAUTION]
> Use a unique `chain_id` for custom devnets to avoid replay risk.

#### L3-Oriented Setup (Starknet as Settlement Layer)

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --name madara \
  --sequencer \
  --preset devnet \
  --settlement-layer starknet \
  --l1-endpoint "$STARKNET_RPC_URL"
```

## Run with Docker

### Manual

```bash
docker pull ghcr.io/madara-alliance/madara:latest
docker run -d \
  -p 9944:9944 \
  -v /var/lib/madara:/tmp/madara \
  --name madara \
  ghcr.io/madara-alliance/madara:latest \
  --name madara \
  --full \
  --network mainnet \
  --l1-endpoint "$ETHEREUM_RPC_URL"
```

```bash
docker logs -f -n 100 madara
```

### Docker Compose via Makefile

```bash
mkdir -p .secrets
echo "$ETHEREUM_RPC_URL" > .secrets/rpc_api.secret

make start
make logs
make stop
make restart
make clean-db
```

To customize runtime args used by compose, edit [`madara-runner.sh`](madara-runner.sh).

## Configuration

### Configuration Precedence

Madara supports three input styles:

1. CLI flags (highest precedence)
2. Environment variables (e.g. `MADARA_RPC_PORT=1111`)
3. Config file (`--config-file <json|toml|yaml>`)

Example:

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --config-file ./configs/args/config.json
```

### CLI Presets

- `--rpc`: externally facing user RPC, admin RPC enabled on localhost
- `--gateway`: externally facing gateway + feeder gateway
- `--warp-update-sender`: source node preset for warp-update migration
- `--warp-update-receiver`: receiver mode for warp-update migration

### Useful Runtime Flags

- `--snap-sync`: batch trie computations for faster sync
- `--disable-tries`: fastest sync path, but no storage-proof support
- `--sync-stop-at <height>`: stop syncing at a specific block
- `--stop-on-sync`: exit once sync target/head is reached
- `--enable-native-execution true`: turn on Cairo Native execution
- `--rpc-unsafe`: enable dangerous admin methods (requires `--rpc-admin`)
- `--settlement-layer {eth|starknet}`: choose settlement backend
- `--external-db-enabled --external-db-mongodb-uri ...`: enable MongoDB external mempool outbox

## RPC and Interactions

Madara supports Starknet JSON-RPC methods through HTTP and WebSocket on the same user RPC port (default `9944`).

### User RPC Endpoints

- Default route (latest): `http://localhost:9944/rpc/v0_10_0/`
- Legacy compatibility routes:
  - `http://localhost:9944/rpc/v0_7_1/`
  - `http://localhost:9944/rpc/v0_8_1/`
  - `http://localhost:9944/rpc/v0_9_0/`
  - `http://localhost:9944/rpc/v0_10_0/`

### Admin RPC Endpoints

- Disabled by default; enable with `--rpc-admin`
- Default port: `9943`
- Versioned path: `http://localhost:9943/rpc/v0_1_0/`

> [!CAUTION]
> Admin methods include operational and state-mutating actions. Keep admin RPC private unless you have proper network controls.

### Supported Method Families

Across supported user RPC versions, Madara provides:

- Read methods (`starknet_getBlockWithTxs`, `starknet_getStateUpdate`, `starknet_getStorageProof`, ...)
- Trace methods (`starknet_traceTransaction`, `starknet_traceBlockTransactions`, `starknet_simulateTransactions`)
- Write methods (`starknet_addInvokeTransaction`, `starknet_addDeclareTransaction`, `starknet_addDeployAccountTransaction`)
- WebSocket subscriptions (`starknet_subscribeNewHeads`, `starknet_subscribeEvents`, `starknet_subscribeTransactionStatus`, `starknet_subscribePendingTransactions`, `starknet_unsubscribe`)

Recent additions/fixes include:

- `starknet_getMessagesStatus` in `v0.9.0` and `v0.10.0`
- `traceBlockTransactions` alignment with Starknet block_id behavior

### Discover Methods Dynamically

Use `rpc_methods` to inspect everything exposed by your running endpoint:

```bash
curl --location 'http://localhost:9944/rpc/v0_10_0/' \
  --header 'Content-Type: application/json' \
  --data '{
    "jsonrpc": "2.0",
    "method": "rpc_methods",
    "params": [],
    "id": 1
  }' | jq --sort-keys
```

## Sync and Migration Strategy

### Sync Modes

1. Standard sync (default): full trie/state progression per block
2. SnapSync (`--snap-sync`): faster sync by batching trie computations
3. No-tries sync (`--disable-tries`): highest speed, but no storage-proof service

> [!IMPORTANT]
> SnapSync and no-tries modes trade off historical proof coverage. If you need broad historical storage-proof support, tune trie-log/snapshot retention and avoid no-tries mode.

### Database Migration Lifecycle

Madara now performs automatic schema migration on startup.

Current DB version metadata:

- Current schema version: `12`
- Minimum migratable version: `8`
- Version source: [`.db-versions.yml`](.db-versions.yml)

Migration safety behavior includes:

- Pre-migration checkpoint backup (`backup_pre_migration/`)
- Locking (`.db-migration.lock`) to avoid concurrent migration
- Resume support (`.db-migration-state`) for interrupted runs

You can skip backup creation with `--skip-migration-backup` only if you already have external backup/snapshot guarantees.

### Warp Update Workflow

Use warp update when you want fast local state transfer from a healthy node instead of full re-sync from genesis.

#### Sender

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --name sender \
  --full \
  --network mainnet \
  --l1-endpoint "$ETHEREUM_RPC_URL" \
  --warp-update-sender
```

#### Receiver

```bash
cargo run --manifest-path madara/Cargo.toml --bin madara --release -- \
  --name receiver \
  --full \
  --network mainnet \
  --base-path /tmp/madara_new \
  --warp-update-receiver \
  --warp-update-shutdown-receiver
```

Custom ports are available via `--warp-update-port-rpc` and `--warp-update-port-fgw`.

## Repository Components

- Node runtime and client crates: [`madara/`](madara/)
- Orchestrator (proof/data/settlement workflow): [`orchestrator/`](orchestrator/README.md)
- Observability dashboards and setup: [`observability/`](observability/README.md)
- Kubernetes deployment assets: [`infra/k8s/`](infra/k8s/README.md)

## Get in Touch

### Contributing

Open an issue first (bug/feature templates are under [`.github/ISSUE_TEMPLATE/`](.github/ISSUE_TEMPLATE/)), then submit a PR using [the PR template](.github/PULL_REQUEST_TEMPLATE.md).

### Partnerships and Support

Reach the Madara team on [Telegram](https://t.me/madara-alliance).

### License

Madara is licensed under [Apache-2.0](LICENSE).
