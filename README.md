<!-- markdownlint-disable -->
<div align="center">
  <img src="https://github.com/keep-starknet-strange/madara-branding/blob/main/logo/PNGs/Madara%20logomark%20-%20Red%20-%20Duotone.png?raw=true" width="500">
</div>

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/madara-alliance/madara)
[![Workflow - Push](https://github.com/madara-alliance/madara/actions/workflows/release-publish.yml/badge.svg)](https://github.com/madara-alliance/madara/actions/workflows/push.yml)
[![Project license](https://img.shields.io/github/license/madara-alliance/madara.svg?style=flat-square)](LICENSE)
[![Pull Requests welcome](https://img.shields.io/badge/PRs-welcome-ff69b4.svg?style=flat-square)](https://github.com/madara-alliance/madara/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
[![Follow on Twitter](https://img.shields.io/twitter/follow/madara-alliance?style=social)](https://twitter.com/madara-alliance)
[![GitHub Stars](https://img.shields.io/github/stars/madara-alliance/madara?style=social)](https://github.com/madara-alliance/madara)

# 🥷 Madara: Starknet Client

Madara is a powerful Starknet client written in Rust.

## Table of Contents

- ⬇️ [Installation](#%EF%B8%8F-installation)
  - [Run from Source](#run-from-source)
  - [Run with Docker](#run-with-docker)
- ⚙️ [Configuration](#%EF%B8%8F-configuration)
  - [Basic Command-Line Options](#basic-command-line-options)
  - [Environment variables](#environment-variables)
  - [Gateway Rate Limits During Sync](#gateway-rate-limits-during-sync)
- 🌐 [Interactions](#-interactions)
  - [Supported JSON-RPC Methods](#supported-json-rpc-methods)
  - [Madara-specific JSON-RPC Methods](#madara-specific-json-rpc-methods)
  - [Example of Calling a JSON-RPC Method](#example-of-calling-a-json-rpc-method)
- 📚 [Database Migration](#-database-migration)
  - [Database Version Management](#database-version-management)
  - [Warp Update](#warp-update)
  - [Running without `--warp-update-sender`](#running-without---warp-update-sender)
- ✅ [Supported Features](#-supported-features)
  - [Starknet Compliant](#starknet-compliant)
  - [Feeder-Gateway State Synchronization](#feeder-gateway-state-synchronization)
  - [Mainnet Full-Node Bootstrap](#mainnet-full-node-bootstrap)
  - [State Commitment Computation](#state-commitment-computation)
  - [SnapSync](#snapsync)
  - [Mainnet Full-Node Bootstrap](#mainnet-full-node-bootstrap)
  - [Cairo Native Execution](#cairo-native-execution)
  - [L3 Support](#l3-support)
  - [Automatic Database Migrations](#automatic-database-migrations)
- 💬 [Get in touch](#-get-in-touch)
  - [Contributing](#contributing)
  - [Partnerships](#partnerships)

## ⬇️ Installation

[⬅️ back to top](#-madara-starknet-client)

> [!TIP]
> For an easier time setting up machine for local development, consult [Using Dev Containers](.devcontainer/README.md).

### Run from Source

#### 1. Install dependencies

Ensure you have all the necessary dependencies available on your host system.

| Dependency | Version    | Installation                                                                        |
| ---------- | ---------- | ----------------------------------------------------------------------------------- |
| Rust       | rustc 1.89 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`                   |
| Clang      | Latest     | `sudo apt-get install clang`                                                        |
| Openssl    | 0.10       | `sudo apt install openssl`                                                          |
| LLVM       | 19         | `make install-llvm19 [SUDO=sudo]` (Ubuntu/Debian) or `brew install llvm@19` (macOS) |

> [!IMPORTANT]
> Source builds require LLVM 19 (`llvm-config-19`) for Cairo Native and the
> `tblgen` build script. Without it, the build fails late with an error like
> `failed to find correct version (19.x.x) of llvm-config (found 18.1.3)`.
> On Ubuntu/Debian, run `make install-llvm19 [SUDO=sudo]` before building. If
> the build scripts still cannot locate LLVM 19, export
> `MLIR_SYS_190_PREFIX`, `LLVM_SYS_191_PREFIX`, and `TABLEGEN_190_PREFIX` to
> your LLVM 19 prefix (`/usr/lib/llvm-19` on Ubuntu/Debian,
> `$(brew --prefix llvm@19)` on macOS).

Once all dependencies are satisfied, you can clone the Madara repository:

```bash
cd <your-destination-path>
git clone https://github.com/madara-alliance/madara
cd madara
```

#### 2. Build Madara

> [!TIP]
> Build scripts normally fetch the published contract artifacts automatically.
> If you need to regenerate local artifacts, run `make artifacts` before
> building. This requires Docker.

You can choose between different build modes:

- **Debug** (low performance, fastest builds, _for testing purposes only_):

  ```bash
  cargo build
  ```

- **Release** (fast performance, slower build times):

  ```bash
  cargo build --release
  ```

- **Production** (fastest performance, _very slow build times_):

  ```bash
  cargo build --profile=production
  ```

#### 3. Run Madara

Start the Madara client with a basic set of arguments depending on your chosen mode:

> [!NOTE]
> Head to the [Configuration](#%EF%B8%8F-configuration) section to learn more about
> customizing your node.

#### Full Node

Synchronizes the state of the chain from genesis.

```bash
cargo run --bin madara --release --        \
  --name Madara               \
  --full                      \
  --base-path /var/lib/madara \
  --network mainnet           \
  --l1-endpoint ${ETHEREUM_API_URL}
```

The user JSON-RPC endpoint is enabled on localhost by default. For a private
full node, omit `--rpc`; set `--rpc-port` only if you need a non-default local
port.

#### Sequencer

Produces new blocks for other nodes to synchronize.

```bash
cargo run --bin madara --release --        \
  --name Madara               \
  --sequencer                 \
  --base-path /var/lib/madara \
  --preset sepolia            \
  --l1-endpoint ${ETHEREUM_API_URL}
```

#### Devnet

A node in a private local network.

```bash
 cargo run --bin madara --release --    \
  --name Madara            \
  --devnet                 \
  --base-path ../madara_db \
  --chain-config-override=chain_id=MY_CUSTOM_DEVNET
```

> [!CAUTION]
> Make sure to use a unique `chain_id` for your devnet to avoid potential replay
> attacks in other chains with the same chain id!

#### 4. Presets

You can use cli presets for certain common node configurations. The `--rpc`
preset is for public RPC provider mode: it exposes user RPC on `0.0.0.0`,
enables admin RPC on localhost, and allows all CORS origins. Do not use it for a
private/local-only full node:

```bash
cargo run --bin madara --release -- \
   --name Madara       \
   --full              \
   --preset mainnet    \
   --rpc
```

...or the madara [feeder gateway](#feeder-gateway-state-synchronization):

```bash
cargo run --bin madara --release -- \
   --name Madara       \
   --full              \
   --preset mainnet    \
   --gateway
```

---

### Run with Docker

#### 1. Manual Setup

| Dependency | Version | Installation                                                     |
| ---------- | ------- | ---------------------------------------------------------------- |
| Docker     | Latest  | [Official instructions](https://docs.docker.com/engine/install/) |

Once you have Docker installed, you will need to pull the madara image from
the github container registry (ghr):

```bash
docker pull ghcr.io/madara-alliance/madara:latest
docker tag ghcr.io/madara-alliance/madara:latest madara:latest
docker rmi ghcr.io/madara-alliance/madara:latest
```

You can then launch madara as follows:

```bash
docker run -d                    \
  -p 9944:9944                   \
  -v /var/lib/madara:/tmp/madara \
  --name Madara                  \
  madara:latest                  \
  --name Madara                  \
  --full                         \
  --network mainnet              \
  --l1-endpoint ${ETHEREUM_API_URL}
```

To display the node's logs, you can use:

```bash
docker logs -f -n 100 Madara
```

> [!WARNING]
> Make sure to change the volume `-v` of your container if ever you update
> `--base-path`.

#### 2. Using the project Makefile

Alternatively, you can use the provided Makefile and `compose.yaml` to
simplify this process.

| Dependency     | Version | Installation                                                      |
| -------------- | ------- | ----------------------------------------------------------------- |
| Docker Compose | Latest  | [Official instructions](https://docs.docker.com/compose/install/) |
| Gnu Make       | Latest  | `sudo apt install make`                                           |

Once you have all the dependencies installed, start by saving your rpc key
to a `.secrets` folder:

```bash
mkdir .secrets
echo "${ETHEREUM_API_URL}" > .secrets/rpc_api.secret
```

Then, run madara with the following commands:

```bash
make start    # This will automatically pull the madara image if not available
make logs     # Displays the last 100 lines of logs
make stop     # Stop the madara node
make clean-db # Removes the madara db, including files on the host
make restart  # Restarts the madara node
```

> [!IMPORTANT]
> By default, `make start` will try and restart Madara indefinitely if it is
> found to be unhealthy using [docker autoheal](https://github.com/willfarrell/docker-autoheal).
> This is done by checking the availability of `http://localhost:9944/health`,
> which means your container will be marked as `unhealthy` and restart if you
> have disabled the RPC service! You can run `watch docker ps` to monitor the
> health of your containers.

To change runtime arguments, you can update the script in `madara-runner.sh`:

```bash
#!/bin/sh
export RPC_API_KEY=$(cat $RPC_API_KEY_FILE)

./madara                   \
  --name madara            \
  --network mainnet        \
  --rpc-external           \
  --rpc-cors all           \
  --full                   \
  --l1-endpoint $RPC_API_KEY
```

For more information, run:

```bash
make help
```

> [!TIP]
> When running Madara from a docker container, make sure to set options such
> as `--rpc-external`, `--gateway-external` and `--rpc-admin-external` so as
> to be able to access these services from outside the container.

## ⚙️ Configuration

[⬅️ back to top](#-madara-starknet-client)

For a comprehensive list of all command-line options, check out:

```bash
cargo run --bin madara -- --help
```

Or if you are using docker, simply:

```bash
docker run madara:latest --help
```

---

### Basic Command-Line Options

Here are some recommended options to get up and started with your Madara client:

| Option                     | About                                                                          |
| -------------------------- | ------------------------------------------------------------------------------ |
| **`--name <NAME>`**        | The human-readable name for this node. It's used as the network node name.     |
| **`--base-path <PATH>`**   | Sets the database location for Madara (default is`/tmp/madara`)                |
| **`--full`**               | The mode of your Madara client (either `--sequencer`, `--full`, or `--devnet`) |
| **`--l1-endpoint <URL>`**  | The Layer 1 endpoint the node will verify its state from                       |
| **`--rpc-port <PORT>`**    | The JSON-RPC server TCP port, used to receive requests                         |
| **`--rpc-cors <ORIGINS>`** | Browser origins allowed to make calls to the RPC servers                       |
| **`--rpc-external`**       | Exposes the rpc service on `0.0.0.0`                                           |

---

### Environment Variables

Each cli argument has its own corresponding environment variable you can set to
change its value. For example:

- `MADARA_BASE_PATH=/path/to/db`
- `MADARA_RPC_PORT=1111`

These variables allow you to adjust the node's configuration without using
command-line arguments, which can be useful in CI pipelines or with docker.

### Configuration files

You can load the arguments directly from a file for ease of use.
The supported file formats are `json`, `toml` and `yaml`.
You can find examples on [configs](configs/).

> [!NOTE]
> If the command-line argument is specified then it takes precedent over the
> environment variable.

### Gateway Rate Limits During Sync

During catchup (especially on mainnet), you may see repeated INFO logs like:

```text
⏳ Rate limited, retrying
```

This means the public feeder gateway returned HTTP 429. Madara automatically
pauses gateway requests for the duration indicated by the `Retry-After` header
(10 seconds if absent) and then retries. Sync continues, just slower — no
action is required for correctness, only for speed.

Options to reduce rate limiting:

- **`--gateway-key <API KEY>`** (`MADARA_GATEWAY_KEY`): bypasses gateway
  throttling for operators who have been issued an API key. Keys are issued by
  the gateway operator; Madara does not provide them.
- **`--gateway-url <URL>`** (`MADARA_GATEWAY_URL`): syncs from a custom or
  alternative feeder gateway instead of the default public one. This can also
  point at a local Madara node serving its own feeder gateway — see
  [warp update](#warp-update) (`--warp-update-sender`) for the local-source
  setup.

When reporting rate-limit-related sync performance, please capture: the block
sync rate (blocks/sec), database size growth, a count or sample of the
`Rate limited, retrying` logs over time, and your gateway configuration
(default public gateway vs `--gateway-url`, with or without `--gateway-key`).

## 🌐 Interactions

[⬅️ back to top](#-madara-starknet-client)

Madara supports Starknet JSON-RPC routes `v0.7.1`, `v0.8.1`, `v0.9.0`,
`v0.10.0`, `v0.10.2`, and `v0.10.3`. Method-level availability can vary
depending on current implementation status and runtime
retention/configuration.
The default user RPC version is `v0.10.3`; the explicit route is
`rpc/v0_10_3`.
Legacy user routes are also available under `rpc/v0_7_1`, `rpc/v0_8_1`,
`rpc/v0_9_0`, `rpc/v0_10_0`, and `rpc/v0_10_2`.
Admin RPC methods are exposed under `rpc/v0_1_0` (default port `9943`) when `--rpc-admin` is enabled.
These methods can be categorized into three main types: Read-Only Access Methods,
Trace Generation Methods, and Write Methods. They are accessible through port
**9944** unless specified otherwise with `--rpc-port`.

> [!NOTE]
> User RPC is enabled by default on **localhost** (disable it with
> `--rpc-disable`). The `--rpc` flag is _not_ needed to enable RPC: it is an
> external RPC _provider_ preset that exposes user RPC on `0.0.0.0`, enables
> admin RPC on localhost, and allows all CORS origins. For a private full node,
> omit `--rpc` and just pick a port:
>
> ```bash
> cargo run --bin madara --release -- --full --network mainnet --rpc-port 9944
> ```

> [!TIP]
> You can use the special `rpc_methods` call to view a list of all the methods
> which are available on an endpoint.

---

### Supported JSON-RPC Methods

Here is a list of all the supported methods with their current status:

<details>
  <summary>Read Methods</summary>

| Status | Method                                                                         |
| ------ | ------------------------------------------------------------------------------ |
| ✅     | `starknet_specVersion`                                                         |
| ✅     | `starknet_getBlockWithTxHashes`                                                |
| ✅     | `starknet_getBlockWithTxs`                                                     |
| ✅     | `starknet_getBlockWithReceipts`                                                |
| ✅     | `starknet_getStateUpdate`                                                      |
| ✅     | `starknet_getStorageAt`                                                        |
| ✅     | `starknet_getTransactionStatus`                                                |
| ✅     | `starknet_getTransactionByHash`                                                |
| ✅     | `starknet_getTransactionByBlockIdAndIndex`                                     |
| ✅     | `starknet_getTransactionReceipt`                                               |
| ✅     | `starknet_getClass`                                                            |
| ✅     | `starknet_getClassHashAt`                                                      |
| ✅     | `starknet_getClassAt`                                                          |
| ✅     | `starknet_getBlockTransactionCount`                                            |
| ✅     | `starknet_call`                                                                |
| ✅     | `starknet_estimateFee`                                                         |
| ✅     | `starknet_estimateMessageFee`                                                  |
| ✅     | `starknet_blockNumber`                                                         |
| ✅     | `starknet_blockHashAndNumber`                                                  |
| ✅     | `starknet_chainId`                                                             |
| ✅     | `starknet_syncing`                                                             |
| ✅     | `starknet_getEvents`                                                           |
| ✅     | `starknet_getNonce`                                                            |
| ✅     | `starknet_getCompiledCasm` (v0.8.1+)                                           |
| ✅     | `starknet_getMessagesStatus` (v0.9.0+)                                         |
| ✅     | `starknet_getStorageProof` (v0.8.1+, latest 128 blocks by default) |

</details>

<details>
  <summary>Trace Methods</summary>

| Status | Method                            |
| ------ | --------------------------------- |
| ✅     | `starknet_traceTransaction`       |
| ✅     | `starknet_simulateTransactions`   |
| ✅     | `starknet_traceBlockTransactions` |

</details>

<details>
  <summary>Write Methods</summary>

| Status | Method                                 |
| ------ | -------------------------------------- |
| ✅     | `starknet_addInvokeTransaction`        |
| ✅     | `starknet_addDeclareTransaction`       |
| ✅     | `starknet_addDeployAccountTransaction` |

</details>

<details>
  <summary>Websocket Methods</summary>

| Status | Method                                                  |
| ------ | ------------------------------------------------------- |
| ✅     | `starknet_unsubscribe` (v0.8.1+)                        |
| ✅     | `starknet_subscribeNewHeads` (v0.8.1+)                  |
| ✅     | `starknet_subscribeEvents` (v0.8.1+)                    |
| ✅     | `starknet_subscribeTransactionStatus` (v0.8.1+)         |
| ✅     | `starknet_subscribePendingTransactions` (v0.8.1 only)   |
| ✅     | `starknet_subscribeNewTransactions` (v0.9.0+)           |
| ✅     | `starknet_subscribeNewTransactionReceipts` (v0.9.0+)    |
| ✅     | `starknet_subscriptionReorg` (notification, v0.8.1+)    |

</details>

> [!NOTE]
> Websocket subscriptions are live on `v0.8.1`, `v0.9.0`, `v0.10.0`,
> `v0.10.2`, and `v0.10.3`, including reorg notifications. Each route follows
> its spec version: `starknet_subscribePendingTransactions` is `v0.8.1` only
> and is replaced by `starknet_subscribeNewTransactions` and
> `starknet_subscribeNewTransactionReceipts` from `v0.9.0` onwards.

> [!IMPORTANT]
> `starknet_getStorageProof` is served for the latest 128 blocks by default
> (`rpc_storage_proof_max_distance = 128`, backed by
> `db_max_saved_trie_logs = 10000` and `db_max_kept_snapshots = 32` with a
> snapshot every 5 blocks). Operators can tune these flags to trade disk usage
> for a deeper proof window, or set `rpc_storage_proof_max_distance = 0` to
> only serve proofs at the chain tip. Note that blocks synced through
> `--snap-sync` batches do not retain per-block trie logs and cannot serve
> historical proofs.

> [!IMPORTANT]
> Write methods are forwarded to the Sequencer and are not executed by Madara.
> These might fail if you provide the wrong arguments or in case of a
> conflicting state. Make sure to refer to the
> [Starknet JSON-RPC specs](https://github.com/starkware-libs/starknet-specs)
> for a list of potential errors.

### Madara-specific JSON-RPC Methods

As well as the official RPC methods, Madara also supports its own set of custom
extensions to the starknet specs. These are referred to as `admin` methods and
are exposed on a separate port **9943** unless specified otherwise with
`--rpc-admin-port`.

<details>
  <summary>Write Methods</summary>

| Method                                     | About                                                       |
| ------------------------------------------ | ----------------------------------------------------------- |
| `madara_addDeclareV0Transaction`           | Adds a legacy Declare V0 transaction                        |
| `madara_bypassAddDeclareTransaction`       | Bypasses mempool/validation for Declare transactions        |
| `madara_bypassAddDeployAccountTransaction` | Bypasses mempool/validation for DeployAccount transactions  |
| `madara_bypassAddInvokeTransaction`        | Bypasses mempool/validation for Invoke transactions         |
| `madara_closeBlock`                        | Forces block closure in block production mode               |
| `madara_revertToAndShutdown`               | Reverts chain state to a block hash and shuts down the node |
| `madara_addL1HandlerMessage`               | Pushes an L1 handler message into bypass input              |
| `madara_setCustomBlockHeader`              | Sets custom block header fields for upcoming block          |

</details>

<details>
  <summary>Read Methods</summary>

| Method                          | About                               |
| ------------------------------- | ----------------------------------- |
| `madara_getBlockBuiltinWeights` | Returns builtin weights for a block |

</details>

<details>
  <summary>Status Methods</summary>

| Method                 | About                                                |
| ---------------------- | ---------------------------------------------------- |
| `madara_ping`          | Return the unix time at which this method was called |
| `madara_shutdown`      | Gracefully stops the running node                    |
| `madara_service`       | Sets the status of one or more services              |
| `madara_serviceStatus` | Returns requested and actual service statuses        |

</details>

<details>
  <summary>Websocket Methods</summary>

| Method         | About                                              |
| -------------- | -------------------------------------------------- |
| `madara_pulse` | Periodically sends a signal that the node is alive |

</details>

> [!CAUTION]
> These methods are exposed on `localhost` by default for obvious security
> reasons. You can always expose them externally using `--rpc-admin-external`,
> but be _very careful_ when doing so as you might be compromising your node!
> Madara does not do **any** authorization checks on the caller of these
> methods and instead leaves it up to the user to set up their own proxy to
> handle these situations.

---

### Example of Calling a JSON-RPC Method

You can use any JSON-RPC client to interact with Madara, such as `curl`,
`httpie`, `websocat` or any client sdk in your preferred programming language.
For more detailed information on how to call each method, please refer to the
[Starknet JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).

#### Http RPC

| Dependency | Version | Installation            |
| ---------- | ------- | ----------------------- |
| Curl       | Latest  | `sudo apt install curl` |

Here is an example of how to call a JSON-RPC method using Madara. Before running
the bellow code, make sure you have a node running with rpc enabled on port 9944
(this is the default configuration).

> [!IMPORTANT]
> Madara currently defaults to `v0.10.3` for RPC calls.
> To access specific versions, add `rpc/v*_*_*/` to your RPC URL.
> This also works for websocket methods.

```bash
curl --location 'http://localhost:9944/rpc/v0_10_3/' \
  --header 'Content-Type: application/json' \
  --data '{
    "jsonrpc": "2.0",
    "method": "rpc_methods",
    "params": [],
    "id": 1
  }' | jq --sort-keys
```

You should receive something like the following:

```bash
{
  "id": 1,
  "jsonrpc": "2.0",
  "result": {
    "methods": [
      "rpc/V0_7_1/starknet_addDeclareTransaction",
      "rpc/V0_7_1/starknet_addDeployAccountTransaction",
      "rpc/V0_7_1/starknet_addInvokeTransaction",
      ...
      "rpc/V0_10_3/starknet_traceBlockTransactions",
      "rpc/V0_10_3/starknet_traceTransaction",
      "rpc/V0_10_3/starknet_unsubscribe",
      "rpc/rpc_methods"
    ]
  }
}
```

#### Websocket RPC

| Dependency | Version | Installation                                                                            |
| ---------- | ------- | --------------------------------------------------------------------------------------- |
| Websocat   | Latest  | [Official instructions](https://github.com/vi/websocat?tab=readme-ov-file#installation) |

Websockets methods are enabled by default and are accessible through the same
port as http RPC methods.

You can use websocket transport to call methods, open subscription streams,
and validate responses using `websocat`:

```bash
websocat -v ws://localhost:9944/rpc/v0_10_3
```

> [!TIP]
> Once connected, paste JSON-RPC payloads directly in the terminal. For example:

```bash
{ "jsonrpc": "2.0", "method": "starknet_subscribeNewHeads", "params": {"block_id":"latest"}, "id": 1 }
```

This opens a subscription stream: you receive a subscription id in the
response, followed by `starknet_subscriptionNewHeads` notifications as new
blocks are produced.

## 📚 Database Migration

[⬅️ back to top](#-madara-starknet-client)

### Database Version Management

Madara now performs automatic database schema migration on startup when needed.
This keeps binary and database versions aligned without manual migration steps
in the common case.

Current migration metadata is tracked in [`.db-versions.yml`](.db-versions.yml):

1. Current schema version: `14`
2. Minimum migratable version: `8`
3. Migrations are resumable and protected by migration lock/state files

> [!IMPORTANT]
> Backups are created before migrations by default. Use
> `--skip-migration-backup` only when you already have external backups.

> [!CAUTION]
> Database migrations are forward-oriented. After migrating to a newer schema
> version, you cannot safely reuse the same database with an older Madara
> binary. To roll back, restore from a backup taken before migration.

To migrate your database, you have two options:

1. Start the node and let automatic in-place migration run
2. Use Madara's **warp update** feature (recommended for fast local migration)
3. Re-synchronize from genesis (not recommended)

The warp update feature provides a trusted sync from a local source, offering
better performance than re-synchronizing the entirety of your chain's state
from genesis.

### Warp Update

Warp update requires a working database source for the migration. If you do not
already have one, you can use the following command to generate a sample
database:

```bash
cargo run --bin madara --release --      \
  --name madara             \
  --network mainnet         \
  --full                    \
  --l1-sync-disabled        `# We disable sync, for testing purposes` \
  --sync-stop-at 1000       `# Only synchronize the first 1000 blocks` \
  --stop-on-sync            `# ...and shutdown the node once this is done`
```

To begin the database migration, you will need to start your node with
[admin methods](#madara-specific-json-rpc-methods) and
[feeder gateway](#feeder-gateway-state-synchronization) enabled. This will be
the _source_ of the migration. You can do this with the `--warp-update-sender`
[preset](#4-presets):

```bash
cargo run --bin madara --release -- \
  --name Sender        \
  --full               `# This also works with other types of nodes` \
  --network mainnet    \
  --warp-update-sender \
  --l1-sync-disabled   `# We disable sync, for testing purposes` \
  --l2-sync-disabled
```

> [!TIP]
> Here, we have disabled sync for testing purposes, so the migration only
> synchronizes the blocks that were already present in the source node's
> database. In a production usecase, you most likely want the source node to
> keep synchronizing with an `--l1-endpoint`, that way when the migration is
> complete the receiver is fully up-to-date with any state that might have been
> produced by the chain _during the migration_.

You will then need to start a second node to synchronize the state of your
database:

```bash
cargo run --bin madara --release --            \
  --name Receiver                 \
  --base-path /tmp/madara_new     `# Where you want the new database to be stored` \
  --full                          \
  --network mainnet               \
  --l1-sync-disabled              `# We disable sync, for testing purposes` \
  --warp-update-receiver          \
  --warp-update-shutdown-receiver `# Shuts down the receiver once the migration has completed`
```

This will start generating a new up-to-date database under `/tmp/madara_new`.
Once this process is over, the receiver node will automatically shutdown.

> [!TIP]
> There also exists a `--warp-update-shutdown-sender` option which allows the
> receiver to take the place of the sender in certain limited circumstances.

### Running without `--warp-update-sender`

Up until now we have had to start a node with `--warp-update-sender` to begin
a migration, but this is only a [preset](#4-presets). In a production
environment, you can start your node with the following arguments and achieve
the same results:

```bash
cargo run --bin madara --release --    \
  --name Sender           \
  --full                  `# This also works with other types of nodes` \
  --network mainnet       \
  --feeder-gateway-enable `# The source of the migration` \
  --gateway-port 8080     `# Default port, change as required` \
  --rpc-admin             `# Used to shutdown the sender after the migration` \
  --rpc-admin-port 9943   `# Default port, change as required` \
  --l1-sync-disabled      `# We disable sync, for testing purposes` \
  --l2-sync-disabled
```

`--warp-update-receiver` doesn't override any cli arguments but is still needed
on the receiver end to start the migration. Here is an example of using it with
custom ports:

> [!IMPORTANT]
> If you have already run a node with `--warp-update-receiver` following the
> examples above, remember to delete its database with `rm -rf /tmp/madara_new`.

```bash
cargo run --bin madara --release --            \
  --name Receiver                 \
  --base-path /tmp/madara_new     `# Where you want the new database to be stored` \
  --full                          \
  --network mainnet               \
  --l1-sync-disabled              `# We disable sync, for testing purposes` \
  --warp-update-port-rpc 9943     `# Same as set with --rpc-admin-port on the sender` \
  --warp-update-port-fgw 8080     `# Same as set with --gateway-port on the sender` \
  --feeder-gateway-enable         \
  --warp-update-receiver          \
  --warp-update-shutdown-receiver `# Shuts down the receiver once the migration has completed`
```

## ✅ Supported Features

[⬅️ back to top](#-madara-starknet-client)

### Starknet compliant

Madara supports Starknet JSON-RPC `v0.7.1`, `v0.8.1`, `v0.9.0`, `v0.10.0`,
`v0.10.2`, and `v0.10.3` (default version: `v0.10.3`).
You can find out more in the [interactions](#-interactions) section and the
official Starknet [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).

### Feeder-Gateway State Synchronization

Madara supports its own implementation of the Starknet feeder gateway, which
allows nodes to synchronize state from each other at much faster speeds than
a regular sync.

> [!NOTE]
> Starknet does not currently have a specification for its feeder-gateway
> protocol, so despite our best efforts at output parity, you might still notice
> some discrepancies between official feeder gateway endpoints and our own
> implementation. Please let us know about if you encounter this by
> [raising an issue](https://github.com/madara-alliance/madara/issues/new/choose)

### Mainnet Full-Node Bootstrap

Madara can bootstrap a mainnet full node in a few ways:

| Path               | Use when                                       | Notes                                                                                                                                                |
| ------------------ | ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Full sync          | You need the most complete local history.      | This is the slowest path.                                                                                                                            |
| `--snap-sync`      | You want faster catchup.                       | Storage proofs are not guaranteed for every snap-synced block, and admin revert cannot target blocks before `snap_sync_latest_block`.                |
| Existing Madara DB | You already have a compatible Madara database. | Stop the node that owns the DB, then start Madara with `--base-path` pointing at the same base path. Keep a backup before reusing or migrating a DB. |

If catchup logs `Rate limited, retrying`, the selected gateway is throttling
requests. Use `--gateway-key` only when you have been issued a key for that
gateway, or use `--gateway-url` to point at a gateway source that you operate or
trust. When reporting rate-limit issues, include the Madara commit, full command
line, current block, target block, blocks/sec, DB size, and how often the
rate-limit message appears.

### State Commitment Computation

Madara supports merkelized state commitments through its own implementation of
Besu Bonsai Merkle Tries. See the [bonsai lib](https://github.com/madara-alliance/bonsai-trie).
You can read more about Starknet Block structure and how it affects state
commitment in the [Starknet documentation](https://docs.starknet.io/architecture-and-concepts/network-architecture/block-structure/).

### SnapSync

Madara supports SnapSync (`--snap-sync`) to accelerate state synchronization by
batching trie computations.

SnapSync uses a batched trie-apply path when both conditions hold:

1. `--snap-sync` is enabled
2. The distance to the sync target is `>= 1000` blocks

When the remaining distance is below this threshold, Madara flushes accumulated
state diffs and returns to block-by-block trie updates.

> [!IMPORTANT]
> SnapSync is a performance tradeoff with historical-data implications:
>
> - For blocks synchronized through SnapSync batches, per-block trie logs
>   ("trielogs") are not produced for each intermediate block in that range.
> - As a result, storage proofs are not guaranteed for every block in
>   snap-synced ranges.
> - Reverting into the snap-synced range is blocked by design. The admin revert
>   API rejects targets lower than the recorded `snap_sync_latest_block` because
>   trie data is only available from that boundary onward.

### Mainnet Full-Node Bootstrap

There are several ways to bring up a mainnet full node, with different
speed/data tradeoffs:

| Path                   | How                                               | Pros                                                                                     | Cons                                                                                                                                    |
| ---------------------- | ------------------------------------------------- | ---------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Full-trie from genesis | `--full --network mainnet`                        | Maximum historical trie data; storage proofs and admin revert work for all synced blocks | Slow with high DB growth (one observed run: ~0.4–0.6 blocks/sec and ~180 GiB at block ~64k on a 12 vCPU host)                           |
| SnapSync               | add `--snap-sync`                                 | Much faster catchup (same benchmark: ~3–11 blocks/sec, ~14 GiB at 31k blocks)            | Storage proofs not available for snap-synced ranges; reverting into snap-synced ranges is blocked by design (see [SnapSync](#snapsync)) |
| Warp update            | `--warp-update-sender` / `--warp-update-receiver` | Fast trusted migration from a local source node                                          | Requires an existing synced node (see [Warp Update](#warp-update))                                                                      |
| Custom gateway / key   | `--gateway-url` and/or `--gateway-key`            | Better catchup reliability, fewer rate limits                                            | Requires issued gateway access (see [Gateway Rate Limits During Sync](#gateway-rate-limits-during-sync))                                |

> [!NOTE]
> The benchmark figures above are a single observed datapoint from one
> operator's hardware, not a guarantee. Your numbers will vary with hardware,
> network, and gateway rate limiting.

If you need storage proofs or the ability to revert to arbitrary historical
blocks via the admin API, use full-trie sync from genesis (or warp update from
a full-trie source). Native database snapshots are tracked as future work in
[#194](https://github.com/madara-alliance/madara/issues/194).

### Cairo Native Execution

Madara supports opt-in Cairo Native execution controlled by a single flag:

```bash
--enable-native-execution true
```

By default, this is disabled and Cairo VM execution remains the execution path.

When native execution is enabled:

1. Madara compiles Sierra classes into native artifacts and caches them.
2. Compilation mode is controlled by `--native-compilation-mode`:
   - `async` (default): compile in background, execute immediately with Cairo VM fallback.
   - `blocking`: wait for compilation; compilation failure/timeout fails execution.
3. Native artifacts are cached on disk under `<base-path>/native_classes` and
   reused on restart.
4. Async compilation retries are controlled by `--native-enable-retry` (default:
   `true`).

This means native compilation is resumable in practice through persisted cache
reuse: compiled classes survive restarts, while classes not yet compiled are
compiled on-demand after restart.

### L3 Support

Madara supports running with different settlement layers via
`--settlement-layer`:

- `Eth` (default)
- `Starknet` (L3-oriented deployment mode)

When `--settlement-layer Starknet` is used:

1. `--l1-endpoint` is treated as a Starknet RPC endpoint (not Ethereum RPC).
2. The settlement client switches to Starknet settlement sync.
3. `--l1-gas-price` and `--blob-gas-price` are interpreted in `FRI` (instead
   of `WEI` on Ethereum settlement).
4. Chain execution is marked as L3 (`is_l3 = true`), which changes how
   blockifier treats settlement-layer addresses.

### Automatic Database Migrations

Madara includes an automatic migration system with checkpoint backups and resume
support to simplify upgrades across database schema versions.

## 💬 Get in touch

[⬅️ back to top](#-madara-starknet-client)

### Contributing

Start with an issue using the templates under [`.github/ISSUE_TEMPLATE/`](.github/ISSUE_TEMPLATE/),
then open a pull request using [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md).

### Partnerships

To establish a partnership with the Madara team, or if you have any suggestions or
special requests, feel free to reach us on [Telegram](https://t.me/madara-alliance).

### License

Madara is open-source software licensed under the
[Apache-2.0 License](https://github.com/madara-alliance/madara/blob/main/LICENSE).
