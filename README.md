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

# 🥷 Madara: Starknet Client

Madara is a powerful Starknet client written in Rust.

## Table of Contents

- ⬇️ Installation
  - [Run from Source](#run-from-source)
  - [Run with Docker](#run-with-docker)
- ⚙️ Configuration
  - [Basic Command-Line Options](#basic-command-line-options)
  - [Advanced Command-Line Options](#advanced-command-line-options)
- 🌐 Interactions
  - [Supported JSON-RPC Methods](#supported-json-rpc-methods)
  - [Example of Calling a JSON-RPC Method](#example-of-calling-a-json-rpc-method)
- Supported Features
- 👍 Contribute

## ⬇️ Installation

### Run from Source

1. **Install dependencies**

   Ensure you have the necessary dependencies:

   | Dependency | Version    | Installation                                                                             |
   | ---------- | ---------- | ---------------------------------------------------------------------------------------- |
   | Rust       | rustc 1.81 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`                        |
   | Clang      | Latest     | `sudo apt-get install clang`                                                             |
   | Scarb      | v2.8.2     | `curl --proto '=https' --tlsv1.2 -sSf https://docs.swmansion.com/scarb/install.sh \| sh` |

   Clone the Madara repository:

   ```bash
   cd <your-destination-path>
   git clone https://github.com/madara-alliance/madara .
   ```

2. **Build Program**

   Choose between different build modes:

   - **Debug** (fastest build mode, but lower performance, for testing purposes only):

     ```bash
     cargo build
     ```

   - **Release** (recommended build mode):

     ```bash
     cargo build --release
     ```

   - **Production** (recommended for production performance):

     ```bash
     cargo build --profile=production
     ```

3. **Run Madara**

   Start the Madara client with a basic set of arguments depending on your chosen mode:

   **Full Node**

   ```bash
   cargo run --release -- \
     --name Madara \
     --full \
     --base-path /var/lib/madara \
     --network mainnet \
     --l1-endpoint ${ETHEREUM_API_URL}
   ```

   **Sequencer**

   ```bash
   cargo run --release -- \
     --name Madara \
     --sequencer \
     --base-path /var/lib/madara \
     --preset test \
     --l1-endpoint ${ETHEREUM_API_URL}
   ```

   **Devnet**

   ```bash
   cargo run --release -- \
     --name Madara \
     --devnet \
     --base-path /var/lib/madara \
     --preset test
   ```

   > ℹ️ **Info:** We recommend you to head to the [Configuration](https://docs.madara.build/fundamentals/configuration)
   > section to customize your node parameters.
   > ℹ️ **Info:** If you don't have an L1 endpoint URL, we recommend you refer to the relevant
   > section to obtain one.

### Run with Docker

1. **Install Docker**

   Ensure you have Docker installed on your machine. Once you have Docker installed, you can run Madara using the available Docker images.

   ```bash
   docker run -d \
     --name Madara \
     --full
     -p 9944:9944 \
     -v /var/lib/madara:/var/lib/madara \
     madara:latest \
     --base-path /var/lib/madara \
     --network mainnet \
     --l1-endpoint ${ETHEREUM_API_URL}
   ```

   > ℹ️ **Info:** This is a default configuration for a Full Node on Starknet mainnet.
   > For more information on possible configurations, please visit the
   > [Configuration](https://docs.madara.build/fundamentals/configuration) section.
   > ⚠️ **Warning:** Make sure to change the volume `-v` of your container if you change the `--base-path`.
   > ℹ️ **Info:** If you don't have an L1 endpoint URL, we recommend you refer to the relevant section to obtain one.

2. **Check Logs**

   ```bash
   docker logs -f Madara
   ```

   > ℹ️ **Info:** Now you can head to the [Metrics](https://docs.madara.build/monitoring/grafana)
   > section to deploy a Grafana and Prometheus dashboard.

## ⚙️ Configuration

### Command-Line Options

For a comprehensive list of command-line options:

```bash
cargo run -- --help
```

Below are some essential command-line options and a categorized list of advanced configurations:

#### Basic Command-Line Options

Here are the recommended options for a quick and simple configuration of your Madara client:

- **`--name <NAME>`**: The human-readable name for this node. It's used as the network node name.

- **`--base-path <PATH>`**: Set the directory for Starknet data (default is `/tmp/madara`).

- **`--full`**: The mode of your Madara client (either `--sequencer`, `--full`, or `devnet`).

- **`--l1-endpoint <URL>`**: Specify the Layer 1 endpoint the node will verify its state from.

- **`--rpc-port <PORT>`**: Specify the JSON-RPC server TCP port.

- **`--rpc-cors <ORIGINS>`**: Specify browser origins allowed to access the HTTP & WS RPC servers.

- **`--rpc-external`**: Listen to all RPC interfaces. Default is local.

> ℹ️ **Info:** For more information regarding synchronization configuration, please refer to the
> [Configuration](https://docs.madara.build/fundamentals/configuration) section.

### Advanced Command-Line Options

Toggle details for each namespace to view additional settings:

<details>
<summary><strong>Network</strong></summary>

- **`-n, --network <NETWORK>`**: The network type to connect to.

  - [default: mainnet]

  Possible values:

  - `mainnet`: The main network (mainnet). Alias: main
  - `testnet`: The test network (testnet). Alias: sepolia
  - `integration`: The integration network
  - `devnet`: A devnet for local testing

- **`--l1-endpoint <ETHEREUM RPC URL>`**: Specify the Layer 1 RPC endpoint for state verification.

- **`--gateway-key <API KEY>`**: Gateway API key to avoid rate limiting (optional).

- **`--sync-polling-interval <SECONDS>`**: Polling interval in seconds.

  - [default: 4]

- **`--pending-block-poll-interval <SECONDS>`**: Pending block polling interval in seconds.

  - [default: 2]

- **`--no-sync-polling`**: Disable sync polling.

- **`--n-blocks-to-sync <NUMBER OF BLOCKS>`**: Number of blocks to sync, useful for benchmarking.

- **`--unsafe-starting-block <BLOCK NUMBER>`**: Start syncing from a specific block. May cause database inconsistency.

- **`--sync-disabled`**: Disable the sync service.

- **`--sync-l1-disabled`**: Disable L1 sync service.

- **`--gas-price-sync-disabled`**: Disable the gas price sync service.

- **`--gas-price-poll-ms <MILLISECONDS>`**: Interval in milliseconds for the gas price sync service to fetch the gas price.
  - [default: 10000]

</details>

<details>
<summary><strong>RPC</strong></summary>

- **`--rpc-disabled`**: Disable the RPC server.

- **`--rpc-external`**: Listen to all network interfaces.

- **`--rpc-methods <METHOD>`**: RPC methods to expose.

  - [default: auto]

  Possible values:

  - `auto`: Expose all methods if RPC is on localhost, otherwise serve only safe methods.
  - `safe`: Allow only a safe subset of RPC methods.
  - `unsafe`: Expose all RPC methods (even potentially unsafe ones).

- **`--rpc-rate-limit <CALLS/MIN>`**: RPC rate limiting per connection.

- **`--rpc-rate-limit-whitelisted-ips <IP ADDRESSES>`**: Disable RPC rate limiting for specific IP addresses or ranges.

- **`--rpc-rate-limit-trust-proxy-headers`**: Trust proxy headers for disabling rate limiting in reverse proxy setups.

- **`--rpc-max-request-size <MEGABYTES>`**: Maximum RPC request payload size for both HTTP and WebSockets.

  - [default: 15]

- **`--rpc-max-response-size <MEGABYTES>`**: Maximum RPC response payload size for both HTTP and WebSockets.

  - [default: 15]

- **`--rpc-max-subscriptions-per-connection <COUNT>`**: Maximum concurrent subscriptions per connection.

  - [default: 1024]

- **`--rpc-port <PORT>`**: The RPC port to listen on.

  - [default: 9944]

- **`--rpc-max-connections <COUNT>`**: Maximum number of RPC server connections at a given time.

  - [default: 100]

- **`--rpc-disable-batch-requests`**: Disable RPC batch requests.

- **`--rpc-max-batch-request-len <LEN>`**: Limit the max length for an RPC batch request.

- **`--rpc-cors <ORIGINS>`**: Specify browser origins allowed to access the HTTP & WebSocket RPC servers.

- **`--rpc-message-buffer-capacity-per-connection <CAPACITY>`**: Maximum number of messages in memory per connection.
  - [default: 64]

</details>

<details>
<summary><strong>Database</strong></summary>

- **`--base-path <PATH>`**: The path where Madara will store the database.

  - [default: /tmp/madara]

- **`--backup-dir <PATH>`**: Directory for backups.

- **`--backup-every-n-blocks <NUMBER OF BLOCKS>`**: Periodically create a backup.

- **`--restore-from-latest-backup`**: Restore the database from the latest backup version.

</details>

<details>
<summary><strong>Block Production</strong></summary>

- **`--block-production-disabled`**: Disable the block production service.

- **`--devnet`**: Launch in block production mode, with devnet contracts.

- **`--devnet-contracts <DEVNET_CONTRACTS>`**: Create this number of contracts in the genesis block for the devnet configuration.

  - [default: 10]

- **`--override-devnet-chain-id`**: Launch a devnet with a production chain ID.

- **`--authority`**: Enable authority mode; the node will run as a sequencer and try to produce its own blocks.

</details>

<details>
<summary><strong>Metrics</strong></summary>

- **`--telemetry`**: Enable connection to the Madara telemetry server.

- **`--telemetry-url <URL VERBOSITY>`**: The URL of the telemetry server with verbosity level.

  - [default: "wss://starknodes.com/submit 0"]

- **`--prometheus-port <PORT>`**: The port used by the Prometheus RPC service.

  - [default: 9615]

- **`--prometheus-external`**: Listen on all network interfaces for Prometheus.

- **`--prometheus-disabled`**: Disable the Prometheus service.

</details>

> ℹ️ **Info:** Note that not all parameters may be referenced here.
> Please refer to the `cargo run -- --help` command for the full list of parameters.

### Environment Variables

Set up your node's environment variables using the `MADARA_` prefix. For example:

- `MADARA_BASE_PATH=/path/to/data`
- `MADARA_RPC_PORT=1111`

These variables allow you to adjust the node's configuration without using command-line arguments. If the command-line
argument is specified then it takes precedent over the environment variable.

> [!CAUTION]
> Environment variables can be visible beyond the current process and are not
> encrypted. You should take special care when setting _secrets_ through
> environment variables, such as `MADARA_L1_ENDPOINT` or `MADARA_GATEWAY_KEY`

### Configuration File

You can use a JSON, TOML, or YAML file to structure your configuration settings.
Specify your configuration file on startup with the `-c` option. Here's a basic example in JSON format:

```json
{
  "name": "Deoxys",
  "base_path": "../deoxys-db",
  "network": "mainnet",
  "l1_endpoint": "l1_key_url",
  "rpc_port": 9944,
  "rpc_cors": "*",
  "rpc_external": true,
  "prometheus_external": true
}
```

> 💡 **Tip:** Review settings carefully for optimal performance and refer to Starknet's
> official documentation for detailed configuration guidelines.

Always test your configuration in a non-production environment before rolling it out to a live node to prevent downtime
and other potential issues.

> ℹ️ **Info:** For a custom chain configuration, you can refer to the configuration section of chain operator deployments.

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

> ℹ️ **Info:** Madara currently supports the latest [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs) up to version v0.7.1.

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

## 📊 Analytics

Madara comes packed with OTEL integration, supporting export of traces, metrics and logs.

- OTEL version `v0.25.0`
- `Trace` and `Logs` are implemented using [tokio tracing](https://github.com/tokio-rs/tracing).
- `Metric` uses OTEL provided metrics.

### Basic Command-Line Option

- **`--analytics-collection-endpoint <URL>`**: Endpoint for OTLP collector, if not provided then OTLP is not enabled.
- **`--analytics-log-level <Log Level>`**: Picked up from `RUST_LOG` automatically, can be provided using this flag as well.
- **`--analytics-service-name <Name>`**: Allows to customize the collection service name.

#### Setting up Signoz

- Signoz Dashboard JSONs are provided at `infra/Signoz/dashboards`.
- Signoz Docker Standalone can be setup following this [guide](https://signoz.io/docs/install/docker/).
- Ensure to configure the correct service_name after importing the json to the dashboard.

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

For guidelines on how to contribute to Madara, please see the [Contribution Guidelines](https://github.com/madara-alliance/madara/blob/main/CONTRIBUTING.md).

## 🤝 Partnerships

To establish a partnership with the Madara team, or if you have any suggestions or
special requests, feel free to reach us on [Telegram](https://t.me/madara-alliance).

## ⚠️ License

Madara is open-source software licensed under the
[Apache-2.0 License](https://github.com/madara-alliance/madara/blob/main/LICENSE).
