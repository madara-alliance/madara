<!-- markdownlint-disable -->
<div align="center">
  <img src="https://github.com/keep-starknet-strange/madara-branding/blob/main/logo/PNGs/Madara%20logomark%20-%20Red%20-%20Duotone.png?raw=true" width="500">
</div>
<div align="center">
<br />
<!-- markdownlint-restore -->
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/madara-alliance/madara)
[![Workflow - Push](https://github.com/madara-alliance/madara/actions/workflows/push.yml/badge.svg)](https://github.com/madara-alliance/madara/actions/workflows/push.yml)
[![Project license](https://img.shields.io/github/license/madara-alliance/madara.svg?style=flat-square)](LICENSE)
[![Pull Requests welcome](https://img.shields.io/badge/PRs-welcome-ff69b4.svg?style=flat-square)](https://github.com/madara-alliance/madara/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
<a href="https://twitter.com/madara-alliance">
<img src="https://img.shields.io/twitter/follow/madara-alliance?style=social"/> </a>
<a href="https://github.com/madara-alliance/madara">
<img src="https://img.shields.io/github/stars/madara-alliance/madara?style=social"/>
</a>

</div>

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
| Rust       | rustc 1.81 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Clang      | Latest     | `sudo apt-get install clang`                                      |
| Openssl    | 0.10       | `sudo apt install openssl`                                        |

Once all dependencies are satisfied, you can clone the Madara repository:

```bash
cd <your-destination-path>
git clone https://github.com/madara-alliance/madara .
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
   --fgw
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
to a `.secrets` forlder:

```bash
mkdir .secrets
echo *** .secrets/rpc_api.secret
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
cargo run -- --help
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

Madara fully supports all the JSON-RPC methods as of the latest version of the
Starknet mainnet official [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).
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
| ✅     | `starknet_getCompiledCasm` (v0.8.0)        |
| 🚧     | `starknet_getMessageStatus` (v0.8.0)       |
| 🚧     | `starknet_getStorageProof` (v0.8.0)        |

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
| ✅     | `starknet_unsubscribe` (v0.8.0)                  |
| ✅     | `starknet_subscribeNewHeads` (v0.8.0)            |
| ✅     | `starknet_subscribeEvents` (v0.8.0)              |
| ❌     | `starknet_subscribeTransactionStatus` (v0.8.0)   |
| ❌     | `starknet_subscribePendingTransactions` (v0.8.0) |
| ❌     | `starknet_subscriptionReorg` (v0.8.0)            |

</details>

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

| Method                           | About                                             |
| -------------------------------- | ------------------------------------------------- |
| `madara_addDeclareV0Transaction` | Adds a legacy Declare V0 Transaction to the state |

</details>

<details>
  <summary>Status Methods</summary>

| Method            | About                                                |
| ----------------- | ---------------------------------------------------- |
| `madara_ping`     | Return the unix time at which this method was called |
| `madara_shutdown` | Gracefully stops the running node                    |
| `madara_service`  | Sets the status of one or more services              |

</details>

<details>
  <summary>Websocket Methods</summary>

| Method         | About                                              |
| -------------- | -------------------------------------------------- |
| `madara_pulse` | Periodically sends a signal that the node is alive |

</details>

> [!CAUTION]
> These methods are exposed on `locahost` by default for obvious security
> reasons. You can always exposes them externally using `--rpc-admin-external`,
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
> Madara currently defaults to `v0.7.1` for its rpc calls. To access methods
> in other or more recent versions, add `rpc/v*_*_*/` to your rpc url. This
> Also works for websocket methods.

```bash
curl --location 'localhost:9944'/v0_7_1/    \
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
      "rpc/V0_8_0/starknet_traceBlockTransactions",
      "rpc/V0_8_0/starknet_traceTransaction",
      "rpc/V0_8_0/starknet_unsubscribe",
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
port as http RPC methods. Here is an example of how to call a JSON-RPC method
using `websocat`.

```bash
(echo '{"jsonrpc":"2.0","method":"starknet_subscribeNewHeads","params":{"block_id":"latest"},"id":1}'; cat -) | \
websocat -v ws://localhost:9944/rpc/v0_8_0
```

> [!TIP]
> This command and the strange use of `echo` in combination with `cat` is just a
> way to start a websocket stream with `websocat` while staying in interactive
> mode, meaning you can still enter other websocket requests.

This will display header information on each new block synchronized. Use
`Ctrl-C` to stop the subscription. Alternatively, you can achieve the same
result more gracefully by calling `starknet_unsubscribe`. Paste the following
into the subscription stream:

```bash
{ "jsonrpc": "2.0", "method": "starknet_unsubscribe", "params": ["your-subscription-id"], "id": 1 }
```

Where `you-subscription-id` corresponds to the value of the `subscription` field
which is returned with each websocket response.

## 📚 Database Migration

[⬅️ back to top](#-madara-starknet-client)

### Database Version Management

The database version management system ensures compatibility between Madara's
binary and database versions.
When you encounter a version mismatch error, it means your database schema needs
to be updated to match your current binary version.

When you see:

```console
Error: Database version 41 is not compatible with current binary. Expected version 42
```

This error indicates that:

1. Your current binary requires database version 42
2. Your database is still at version 41
3. Migration is required before you can continue

> [!IMPORTANT]
> Don't panic! Your data is safe, but you need to migrate it before continuing.

To migrate your database, you have two options:

1. Use Madara's **warp update** feature (recommended)
2. Re-synchronize from genesis (not recommended)

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
  --n-blocks-to-sync 1000   `# Only synchronize the first 1000 blocks` \
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
> There also exists a `--warp-update--shutdown-sender` option which allows the
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

Madara is compliant with the latest `v0.13.2` version of Starknet and `v0.7.1`
JSON-RPC specs. You can find out more about this in the [interactions](#-interactions)
section or at the official Starknet [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).

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

## 💬 Get in touch

[⬅️ back to top](#-madara-starknet-client)

### Contributing

For guidelines on how to contribute to Madara, please see the [Contribution Guidelines](https://github.com/madara-alliance/madara/blob/main/CONTRIBUTING.md).

### Partnerships

To establish a partnership with the Madara team, or if you have any suggestions or
special requests, feel free to reach us on [Telegram](https://t.me/madara-alliance).

### License

Madara is open-source software licensed under the
[Apache-2.0 License](https://github.com/madara-alliance/madara/blob/main/LICENSE).
