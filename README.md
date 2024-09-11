<!-- markdownlint-disable -->
<div align="center">
  <img src="https://github.com/keep-starknet-strange/madara-branding/blob/main/logo/PNGs/Madara%20logomark%20-%20Red%20-%20Duotone.png?raw=true" width="500">
</div>
<div align="center">
<br />
<!-- markdownlint-restore -->

[![Workflow - Push](https://github.com/madara-alliance/madara/actions/workflows/push.yml/badge.svg)](https://github.com/madara-alliance/madara/actions/workflows/push.yml)
[![Project license](https://img.shields.io/github/license/madara-alliance/madara.svg?style=flat-square)](LICENSE)
[![Pull Requests welcome](https://img.shields.io/badge/PRs-welcome-ff69b4.svg?style=flat-square)](https://github.com/madara-alliance/madara/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
<a href="https://twitter.com/madara-alliance">
<img src="https://img.shields.io/twitter/follow/madara-alliance?style=social"/> </a>
<a href="https://github.com/madara-alliance/madara">
<img src="https://img.shields.io/github/stars/madara-alliance/madara?style=social"/>
</a>

</div>

# 🥷 Madara: Starknet client

Madara is a powerfull Starknet hybrid client written in Rust.

## Table of Contents

- ⬇️ Installation
  - [Run from Source](#run-from-source)
  - [Run with Docker](#run-with-docker)
  - [Run with Docker Compose](#run-with-docker-compose)
- ⚙️ Configuration
  - [Basic Command-Line Options](#basic-command-line-options)
  - [Advanced Command-Line Options](#advanced-command-line-options)
- 📸 Snapshots
- 🌐 Interactions
  - [Supported JSON-RPC Methods](#supported-json-rpc-methods)
  - [Example of Calling a JSON-RPC Method](#example-of-calling-a-json-rpc-method)
- Supported Features
- 👍 Contribute

## ⬇️ Installation

### Run from Source

1. **Install dependencies**

   Ensure you have the necessary dependencies:

   ```sh
   sudo apt-get update && sudo apt-get install -y \
     clang \
     protobuf-compiler \
     build-essential
   ```

   Install Rust:

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s
   ```

   Clone the Madara repository:

   ```sh
   cd <your-destination-path>
   git clone https://github.com/madara-alliance/madara .
   ```

2. **Run Madara**

   Start the Madara client with synchronization to Starknet mainnet:

   ```sh
   cargo run --release -- \
     --name madara \
     --base-path ../madara-db \
     --network main \
     --l1-endpoint ${ETHEREUM_API_URL} \
   ```

### Run with Docker

1 **Run docker image**

To run Madara with Docker, use the following command:

```sh
docker run -d \
    --name madara \
    -p 9944:9944 \
    -v /var/lib/madara:/var/lib/madara \
    madara:latest \
    --base-path ../madara-db \
    --network main \
    --l1-endpoint <rpc key> \
```

Check the logs of the running Madara service:

```sh
docker logs -f madara
```

### Run with Docker Compose

1. **Ensure environment variable**

   Set the necessary environment variable:

   ```sh
   export L1_ENDPOINT="your-ethereum-api-url"
   ```

   Or create a `.env` file in the same directory as your `docker-compose.yml` file:

   ```sh
   L1_ENDPOINT=your-ethereum-api-url
   ```

2. **Build and Run the Container**

   Navigate to the directory with your `docker-compose.yml` file and run one of the following commands:

   - Mainnet:

     ```sh
     docker-compose --profile mainnet up -d
     ```

   - Testnet:

     ```sh
     docker-compose --profile testnet up -d
     ```

   Check the logs of the running Madara service:

   ```sh
   docker-compose logs -f madara
   ```

## ⚙️ Configuration

Configuring your Madara node properly ensures it meets your specific needs

### Basic Command-Line Options

Here are the recommended options for a quick and simple configuration of your Madara full node:

- **`--name <NAME>`**: The human-readable name for this node. It's used as the network node name.
- **`--base-path <PATH>`**: Set the directory for Starknet data (default is `/tmp/madara`).
- **`--network <NETWORK>`**: The network type to connect to (`main`, `test`, or `integration`).
- **`--l1-endpoint <URL>`**: Specify the Layer 1 endpoint the node will verify its state from.
- **`--rpc-port <PORT>`**: Specify the JSON-RPC server TCP port.
- **`--rpc-cors <ORIGINS>`**: Specify browser origins allowed to access the HTTP & WS RPC servers.
- **`--rpc-external`**: Listen to all RPC interfaces. Default is local.
- **`--snap <BLOCK_NUMBER>`**: Start syncing from the closest snapshot available for the desired block (default is highest).

### Advanced Command-Line Options

Here are more advanced command-line options, organized by namespace, for running and development purposes:

<details>
<summary>Network</summary>

- **`-n, --network <NETWORK>`**: The network type to connect to (default: `integration`).
- **`--port <PORT>`**: Set the network listening port.
- **`--l1-endpoint <URL>`**: Specify the Layer 1 endpoint the node will verify its state from.
- **`--gateway-key <GATEWAY_KEY>`**: Gateway API key to avoid rate limiting (optional).
- **`--sync-polling-interval <SECONDS>`**: Polling interval in seconds (default: 2).
- **`--no-sync-polling`**: Stop sync polling.
- **`--n-blocks-to-sync <NUMBER>`**: Number of blocks to sync.
- **`--starting-block <BLOCK>`**: The block to start syncing from (make sure to set `--disable-root`).

</details>

<details>
<summary>RPC</summary>

- **`--rpc-external`**: Listen to all RPC interfaces. Note: not all RPC methods are safe to be exposed publicly.
  Use an RPC proxy server to filter out dangerous methods.
- **`--rpc-methods <METHOD_SET>`**: RPC methods to expose (`auto`, `safe`, `unsafe`).
- **`--rpc-max-request-size <SIZE>`**: Set the maximum RPC request payload size in megabytes (default: 15).
- **`--rpc-max-response-size <SIZE>`**: Set the maximum RPC response payload size in megabytes (default: 15).
- **`--rpc-max-subscriptions-per-connection <NUMBER>`**: Set the maximum concurrent subscriptions per connection (default: 1024).
- **`--rpc-port <PORT>`**: Specify JSON-RPC server TCP port.
- **`--rpc-max-connections <NUMBER>`**: Maximum number of RPC server connections (default: 100).
- **`--rpc-cors <ORIGINS>`**: Specify browser origins allowed to access the HTTP & WS RPC servers.

</details>

<details>
<summary>Database</summary>

- **`--base-path <PATH>`**: Specify custom base path (default: `/tmp/madara`).
- **`--snap <BLOCK_NUMBER>`**: Start syncing from the closest snapshot available for the desired block.
- **`--tmp`**: Run a temporary node. A temporary directory will be created and deleted at the end of the process.
- **`--cache`**: Enable caching of blocks and transactions to improve response times.
- **`--db-cache <MiB>`**: Limit the memory the database cache can use.
- **`--trie-cache-size <Bytes>`**: Specify the state cache size (default: 67108864).
- **`--backup-every-n-blocks <NUMBER>`**: Specify the number of blocks after which a backup should be created.
- **`--backup-dir <DIR>`**: Specify the directory where backups should be stored.
- **`--restore-from-latest-backup`**: Restore the database from the latest backup available.

</details>

> ℹ️ **Info:** Note that not all parameters may be referenced here.
> Please refer to the `cargo run -- --help` command for the full list of parameters.

## 📸 Snapshots

Snapshots are under developpement and will be available through the `--snap <block_number>` parameter.

## 🌐 Interactions

Madara fully supports all the JSON-RPC methods as specified in the Starknet mainnet official [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).
These methods can be categorized into three main types: Read-Only Access Methods, Trace Generation Methods, and Write Methods.

### Supported JSON-RPC Methods

Here is a list of all the supported methods with their current status:

<details>
  <summary>Read Methods</summary>

| Status | Method                                     |
| ------ | ------------------------------------------ |
| ✅     | `starknet_specVersion`                     |
| ✅     | `starknet_getBlockWithTxHashes`            |
| ✅     | `starknet_getBlockWithReceipts`            |
| ✅     | `starknet_getBlockWithTxs`                 |
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

> ℹ️ **Info:** Madara currently supports latest [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs) specs up to version v0.7.1

### Example of Calling a JSON-RPC Method

Here is an example of how to call a JSON-RPC method using Madara:

```json
{
  "jsonrpc": "2.0",
  "method": "starknet_getBlockWithTxHashes",
  "params": {
    "block_id": "latest"
  },
  "id": 1
}
```

You can use any JSON-RPC client to interact with the Madara node, such as `curl`, `httpie`,
or a custom client in your preferred programming language.
For more detailed information and examples on each method, please refer to the [Starknet JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).

> ⚠️ **Warning:** Write methods are forwarded to the Sequencer for execution.
> Ensure you handle errors appropriately as per the JSON-RPC schema.

## ✔ Supported Features

Madara offers numerous features and is constantly improving to stay at the cutting edge of Starknet technology.

- **Starknet Version**: `v0.13.2`
- **JSON-RPC Version**: `v0.7.1`
- **Feeder-Gateway State Synchronization**
- **State Commitment Computation**
- **L1 State Verification**
- **Handling L1 and L2 Reorgs**

Each feature is designed to ensure optimal performance and seamless integration with the Starknet ecosystem.

## 👍 Contribute

## 🤝 Partnerships

To establish a partnership with the Kasar team, or if you have any suggestion or
special request, feel free to reach us on [telegram](https://t.me/madara-alliance).

## ⚠️ License

Copyright (c) 2022-present, with the following
[contributors](https://github.com/madara-alliance/madara/graphs/contributors).

Madara is open-source software licensed under the
[Apache-2.0 License](https://github.com/madara-alliance/madara/blob/main/LICENSE).
