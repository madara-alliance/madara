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
    🌐 [Interactions](#-interactions)
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
  - [State Commitment Computation](#state-commitment-computation)
  - [SnapSync](#snapsync)
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

| Dependency | Version    | Installation                                                      |
| ---------- | ---------- | ----------------------------------------------------------------- |
| Rust       | rustc 1.89 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Clang      | Latest     | `sudo apt-get install clang`                                      |
| Openssl    | 0.10       | `sudo apt install openssl`                                        |

Once all dependencies are satisfied, you can clone the Madara repository:

```bash
cd <your-destination-path>
git clone https://github.com/madara-alliance/madara
cd madara
```

#### 2. Build Madara

> [!TIP]
> Ensure `make snos` has been run prior to building Madara.

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

You can use cli presets for certain common node configurations, for example
enabling rpc endpoints:

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

## 🌐 Interactions

[⬅️ back to top](#-madara-starknet-client)

Madara supports Starknet JSON-RPC routes `v0.7.1`, `v0.8.1`, `v0.9.0`, and
`v0.10.0`. Method-level availability can vary depending on current implementation
status and runtime retention/configuration.
The default user RPC route is `rpc/v0_10_0`.
Legacy user routes are also available under `rpc/v0_7_1`, `rpc/v0_8_1`, and `rpc/v0_9_0`.
Admin RPC methods are exposed under `rpc/v0_1_0` (default port `9943`) when `--rpc-admin` is enabled.
These methods can be categorized into three main types: Read-Only Access Methods,
Trace Generation Methods, and Write Methods. They are accessible through port
**9944** unless specified otherwise with `--rpc-port`.

> [!TIP]
> You can use the special `rpc_methods` call to view a list of all the methods
> which are available on an endpoint.

---

### Supported JSON-RPC Methods

Here is a list of all the supported methods with their current status:

<details>
  <summary>Read Methods</summary>

| Status | Method                                     |
| ------ | ------------------------------------------ |
| ✅     | `starknet_specVersion`                     |
| ✅     | `starknet_getBlockWithTxHashes`            |
| ✅     | `starknet_getBlockWithTxs`                 |
| ✅     | `starknet_getBlockWithReceipts`            |
| ✅     | `starknet_getStateUpdate`                  |
| ✅     | `starknet_getStorageAt`                    |
| ✅     | `starknet_getTransactionStatus`            |
| ✅     | `starknet_getTransactionByHash`            |
| ✅     | `starknet_getTransactionByBlockIdAndIndex` |
| ✅     | `starknet_getTransactionReceipt`           |
| ✅     | `starknet_getClass`                        |
| ✅     | `starknet_getClassHashAt`                  |
| ✅     | `starknet_getClassAt`                      |
| ✅     | `starknet_getBlockTransactionCount`        |
| ✅     | `starknet_call`                            |
| ✅     | `starknet_estimateFee`                     |
| ✅     | `starknet_estimateMessageFee`              |
| ✅     | `starknet_blockNumber`                     |
| ✅     | `starknet_blockHashAndNumber`              |
| ✅     | `starknet_chainId`                         |
| ✅     | `starknet_syncing`                         |
| ✅     | `starknet_getEvents`                       |
| ✅     | `starknet_getNonce`                        |
| ✅     | `starknet_getCompiledCasm` (v0.8.1+)       |
| ✅     | `starknet_getMessagesStatus` (v0.9.0+)     |
| ❌     | `starknet_getStorageProof` (v0.8.1+, currently unavailable in default profile) |

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

| Status | Method                                           |
| ------ | ------------------------------------------------ |
| ✅     | `starknet_unsubscribe` (v0.8.1+)                  |
| ❌     | `starknet_subscribeNewHeads` (placeholder)        |
| ❌     | `starknet_subscribeEvents` (placeholder)          |
| ❌     | `starknet_subscribeTransactionStatus` (placeholder) |
| ❌     | `starknet_subscribePendingTransactions` (placeholder) |
| ❌     | `starknet_subscriptionReorg`                       |

</details>

> [!NOTE]
> Subscription methods are currently placeholders and return `UnimplementedMethod`.
> This applies to `v0.8.1`, `v0.9.0`, and `v0.10.0` (which delegates to `v0.9.0`).

> [!IMPORTANT]
> `starknet_getStorageProof` is currently treated as unavailable in the default
> node profile because the required retention is disabled in
> [`configs/args/config.json`](configs/args/config.json):
> `db_max_saved_trie_logs = 0`, `db_max_kept_snapshots = 0`, and
> `rpc_storage_proof_max_distance = 0`. Madara loads this file by default when
> run without explicit CLI/config overrides.

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

| Method                                      | About                                                             |
| ------------------------------------------- | ----------------------------------------------------------------- |
| `madara_addDeclareV0Transaction`            | Adds a legacy Declare V0 transaction                              |
| `madara_bypassAddDeclareTransaction`        | Bypasses mempool/validation for Declare transactions              |
| `madara_bypassAddDeployAccountTransaction`  | Bypasses mempool/validation for DeployAccount transactions        |
| `madara_bypassAddInvokeTransaction`         | Bypasses mempool/validation for Invoke transactions               |
| `madara_closeBlock`                         | Forces block closure in block production mode                     |
| `madara_revertToAndShutdown`                | Reverts chain state to a block hash and shuts down the node       |
| `madara_addL1HandlerMessage`                | Pushes an L1 handler message into bypass input                    |
| `madara_setCustomBlockHeader`               | Sets custom block header fields for upcoming block                |

</details>

<details>
  <summary>Read Methods</summary>

| Method                         | About                                  |
| ------------------------------ | -------------------------------------- |
| `madara_getBlockBuiltinWeights`| Returns builtin weights for a block    |

</details>

<details>
  <summary>Status Methods</summary>

| Method            | About                                                |
| ----------------- | ---------------------------------------------------- |
| `madara_ping`     | Return the unix time at which this method was called |
| `madara_shutdown` | Gracefully stops the running node                    |
| `madara_service`  | Sets the status of one or more services              |
| `madara_serviceStatus` | Returns requested and actual service statuses   |

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
> Madara currently defaults to `v0.10.0` for RPC calls.
> To access specific versions, add `rpc/v*_*_*/` to your RPC URL.
> This also works for websocket methods.

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
      "rpc/V0_10_0/starknet_traceBlockTransactions",
      "rpc/V0_10_0/starknet_traceTransaction",
      "rpc/V0_10_0/starknet_unsubscribe",
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

> [!NOTE]
> Subscription methods are currently placeholders and return
> `UnimplementedMethod` on `v0.8.1`, `v0.9.0`, and `v0.10.0`.
> `starknet_unsubscribe` is available, but active subscription streams are not
> yet available.

You can still use websocket transport to call methods and validate responses
using `websocat`:

```bash
websocat -v ws://localhost:9944/rpc/v0_10_0
```

> [!TIP]
> Once connected, paste JSON-RPC payloads directly in the terminal. For example:

```bash
{ "jsonrpc": "2.0", "method": "starknet_subscribeNewHeads", "params": {"block_id":"latest"}, "id": 1 }
```

This currently returns `UnimplementedMethod` until subscription support is
re-enabled.

## 📚 Database Migration

[⬅️ back to top](#-madara-starknet-client)

### Database Version Management

Madara now performs automatic database schema migration on startup when needed.
This keeps binary and database versions aligned without manual migration steps
in the common case.

Current migration metadata is tracked in [`.db-versions.yml`](.db-versions.yml):

1. Current schema version: `12`
2. Minimum migratable version: `8`
3. Migrations are resumable and protected by migration lock/state files

> [!IMPORTANT]
> Backups are created before migrations by default. Use
> `--skip-migration-backup` only when you already have external backups.

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

Madara supports Starknet JSON-RPC `v0.7.1`, `v0.8.1`, `v0.9.0`, and `v0.10.0`
(default route: `v0.10.0`).
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
