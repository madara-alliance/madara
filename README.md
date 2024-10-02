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
<a href="https://twitter.com/madara_alliance">
<img src="https://img.shields.io/twitter/follow/madara_alliance?style=social"/> </a>
<a href="https://github.com/madara-alliance/madara">
<img src="https://img.shields.io/github/stars/madara-alliance/madara?style=social"/>
</a>

</div>

# 🥷 Madara: Starknet Client

Madara is a powerful Starknet hybrid client written in Rust.

## Table of Contents

- ⬇️ Installation
  - [Run from Source](#run-from-source)
  - [Run with Docker (Recommended)](#run-with-docker-recommended)
  - [Run with Docker Compose](#run-with-docker-compose)
  - [Run with Installation Script](#run-with-installation-script)
- ⚙️ Configuration
  - [Priority of Configuration](#priority-of-configuration)
  - [Command-Line Options](#command-line-options)
    - [Basic Command-Line Options](#basic-command-line-options)
    - [Advanced Command-Line Options by Namespace](#advanced-command-line-options-by-namespace)
  - [Environment Variables](#environment-variables)
  - [Configuration File](#configuration-file)
- 📸 Snapshots
- 🌐 Interactions
  - [Supported JSON-RPC Methods](#supported-json-rpc-methods)
  - [Example of Calling a JSON-RPC Method](#example-of-calling-a-json-rpc-method)
- ✔ Supported Features
- 👍 Contribute
- 🤝 Partnerships
- ⚠️ License

## ⬇️ Installation

In this section, we will guide you through the build and run process so that you can run your own Madara client and query the Starknet blockchain as smoothly as possible.

We have divided this section into three difficulty levels:

- [**Low-level**](#run-from-source) (from source by building the Rust binary locally)
- [**Mid-level**](#run-with-docker-recommended) (from Docker **recommended** via the available Docker images)
- [**High-level**](#run-with-installation-script) (from a high-level interactive menu)

### Run from Source

This installation process will help you build the binary directly from the source code locally on your machine.

1. **Install Dependencies**

   Ensure you have everything needed to complete this tutorial.

   | Dependency | Version    | Installation Command                                                              |
   | ---------- | ---------- | --------------------------------------------------------------------------------- |
   | Rust       | rustc 1.78 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`                 |
   | Clang      | Latest     | `sudo apt-get install clang`                                                      |
   | Scarb      | v2.8.2     | `curl --proto '=https' --tlsv1.2 -sSf https://docs.swmansion.com/scarb/install.sh \| sh` |

2. **Get Code**

   Fetch the code from the official [Madara](https://github.com/madara-alliance/madara) repository in the folder of your choice.

   ```bash
   cd <your-destination-path>
   git clone https://github.com/madara-alliance/madara .
   ```

3. **Build Program**

   Choose one of the following build modes:

   - **Debug** (fastest build mode, but lower performance, for testing purposes only)

     ```bash
     cargo build
     ```

   - **Release** (the recommended build mode)

     ```bash
     cargo build --release
     ```

   - **Production** (the recommended build mode for production performance)

     ```bash
     cargo build --profile=production
     ```

4. **Run Madara**

   Start the Madara client with a basic set of arguments, depending on your chosen mode:

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

   > ℹ️ **Note:** We recommend visiting the [Configuration](#configuration) section to customize your node parameters.

   > ℹ️ **Note:** If you don't have an L1 endpoint URL, we recommend heading to the [Verification](#verification) section to get one.

### Run with Docker (Recommended)

This is the recommended way to easily install and run Madara as it only requires terminal access and Docker installed.

1. **Install Docker**

   Ensure Docker is installed on your system. If not, you can install it by following the instructions for your operating system:

   - **MacOS**:

     Download and install Docker Desktop for Mac from [Docker Hub](https://hub.docker.com/editions/community/docker-ce-desktop-mac/).

   - **Linux**:

     Install Docker using the package manager for your distribution. For example, on Ubuntu:

     ```bash
     sudo apt-get update
     sudo apt-get install \
       ca-certificates \
       curl \
       gnupg \
       lsb-release

     curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg

     echo \
       "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] \
       https://download.docker.com/linux/ubuntu \
       $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null

     sudo apt-get update
     sudo apt-get install docker-ce docker-ce-cli containerd.io
     ```

   - **Windows**:

     Download and install Docker Desktop for Windows from [Docker Hub](https://hub.docker.com/editions/community/docker-ce-desktop-windows/).

2. **Run Docker Image**

   Once you have successfully installed Docker, you can now run Madara using the available Docker images.

   ```bash
   docker run -d \
     --name Madara \
     -p 9944:9944 \
     -v /var/lib/madara:/var/lib/madara \
     madara:latest \
     --base-path /var/lib/Madara \
     --network main \
     --l1-endpoint ${ETHEREUM_API_URL}
   ```

   > ℹ️ **Note:** This is a default configuration. For more information on possible configurations, please visit the [Configuration](#configuration) section.

   > ⚠️ **Warning:** Make sure to change the volume `-v` of your container if you change the `--base-path`.

   If you don't have an L1 endpoint URL, we recommend heading to the [Verification](#verification) section to get one.

3. **Check Logs**

   To check the logs of the running Madara service:

   ```bash
   docker logs -f Madara
   ```

   Now you can head up to the [Metrics](#metrics) section to easily deploy a Grafana and Prometheus dashboard.

### Run with Docker Compose

1. **Prerequisites**

   Ensure you have Docker and Docker Compose installed on your machine.

2. **Prepare the Environment**

   Set the necessary environment variable:

   ```bash
   export ETHEREUM_API_URL="your-ethereum-api-url"
   ```

   Or create a `.env` file in the same directory as your `docker-compose.yml` file:

   ```
   ETHEREUM_API_URL=your-ethereum-api-url
   ```

   > ⚠️ **Note:** When running with `sudo`, environment variables set in the current session might not be carried over. You can pass the variable directly to sudo using the `-E` option to preserve the environment.

   If you don't have an L1 endpoint URL, we recommend heading to the [Verification](#verification) section to get one.

3. **Build and Run the Container**

   Navigate to the directory with your `docker-compose.yml` file and run the following command:

   ```bash
   docker-compose up -d
   ```

   This command will build the Docker image and start the container in detached mode.

4. **Check Logs**

   You can view the logs of the running Madara service using the following command:

   ```bash
   docker-compose logs -f Madara
   ```

   Now you can head up to the [Metrics](#metrics) section to easily deploy a Grafana and Prometheus dashboard.

### Run with Installation Script

This is the highest-level way to install Madara with some custom features. For advanced configuration, we recommend using the source or Docker methods.

1. **Download Script**

   You will need to download and run the installer script using the following command:

   ```bash
   # Installation script command line (to be provided)
   ```

2. **Follow Instructions**

   Follow the interactive instructions provided by the installation script to set up your Madara client.

   > **Note:** This method is under development. For more advanced configurations, please use the source or Docker methods.

> ℹ️ **Note:** Now that you know how to launch a Madara client, you might want to set some parameters to customize it. Therefore, you can go to the following [Configuration](#configuration) section.

## ⚙️ Configuration

Configuring your Madara client properly ensures that it meets your specific needs. Configuration can be done through three main avenues:

- **Command-line arguments**
- **Environment variables**
- **Configuration files**

### Priority of Configuration

> ℹ️ **Note:** Configuration priority is as follows: command-line arguments > environment variables > configuration files. When the same setting is configured in multiple places, the source with the highest priority takes effect.

### Command-Line Options

For a comprehensive list of command-line options:

```bash
cargo run -- --help
```

Below are some essential command-line options and a categorized list of advanced configurations:

#### Basic Command-Line Options

Here are the recommended options for a quick and simple configuration of your Madara client:

- **`--name <NAME>`**: The human-readable name for this node. It's used as the network node name.
- **`--base-path <PATH>`**: Set the directory for Madara data (default is `/tmp/madara`).
- **`--network <NETWORK>`**: The network type to connect to (`main`, `test`, `integration`, or `devnet`).
- **`--l1-endpoint <URL>`**: Specify the Layer 1 endpoint the node will verify its state from.
- **`--rpc-port <PORT>`**: Specify the JSON-RPC server TCP port.
- **`--rpc-cors <ORIGINS>`**: Specify browser origins allowed to access the HTTP & WebSocket RPC servers.
- **`--rpc-external`**: Listen to all RPC interfaces. Default is local.

> ℹ️ **Note:** For more information regarding synchronization configuration, please refer to the next section.

#### Advanced Command-Line Options by Namespace

Toggle details for each namespace to view additional settings:

<details>
<summary><strong>Network</strong></summary>

- **`--network <NETWORK>`**: The network type to connect to.
  - [default: main]

  Possible values:

  - `main`: The main network (mainnet). Alias: mainnet
  - `test`: The test network (testnet). Alias: sepolia
  - `integration`: The integration network
  - `devnet`: A devnet for local testing

- **`--l1-endpoint <ETHEREUM RPC URL>`**: Specify the Layer 1 RPC endpoint for state verification.

- **`--gateway-key <API KEY>`**: Gateway API key to avoid rate limiting (optional).

- **`--sync-polling-interval <SECONDS>`**: Polling interval in seconds. This affects the sync service after catching up with the blockchain tip.
  - [default: 4]

- **`--pending-block-poll-interval <SECONDS>`**: Pending block polling interval in seconds.
  - [default: 2]

- **`--no-sync-polling`**: Disable sync polling. Sync service will not import new blocks after catching up with the blockchain tip.

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

- **`--rpc-max-request-size <MEGABYTES>`**: Maximum RPC request payload size.
  - [default: 15]

- **`--rpc-max-response-size <MEGABYTES>`**: Maximum RPC response payload size.
  - [default: 15]

- **`--rpc-max-subscriptions-per-connection <COUNT>`**: Maximum concurrent subscriptions per connection.
  - [default: 1024]

- **`--rpc-port <PORT>`**: The RPC port to listen on.
  - [default: 9944]

- **`--rpc-max-connections <COUNT>`**: Maximum number of RPC server connections at a given time.
  - [default: 100]

- **`--rpc-disable-batch-requests`**: Disable RPC batch requests.

- **`--rpc-max-batch-request-len <LEN>`**: Limit the max length for an RPC batch request.

- **`--rpc-cors <ORIGINS>`**: Specify browser origins allowed to access the HTTP & WebSocket RPC servers. A comma-separated list of origins, or the special `all` value.

- **`--rpc-message-buffer-capacity-per-connection <CAPACITY>`**: Maximum number of messages in memory per connection.
  - [default: 64]

</details>

<details>
<summary><strong>Database</strong></summary>

- **`--base-path <PATH>`**: The path where Madara will store the database.
  - [default: /tmp/madara]

- **`--backup-dir <PATH>`**: Directory for backups.

- **`--backup-every-n-blocks <NUMBER OF BLOCKS>`**: Periodically create a backup, useful for debugging.

- **`--restore-from-latest-backup`**: Restore the database from the latest backup version.

</details>

<details>
<summary><strong>Block Production</strong></summary>

- **`--block-production-disabled`**: Disable the block production service.

- **`--devnet`**: Launch in block production mode, with devnet contracts.

- **`--devnet-contracts <DEVNET_CONTRACTS>`**: Create this number of contracts in the genesis block for the devnet configuration.
  - [default: 10]

- **`--override-devnet-chain-id`**: Launch a devnet with a production chain ID. This is unsafe because your devnet transactions can be replayed on the actual network.

- **`--authority`**: Enable authority mode; the node will run as a sequencer and try to produce its own blocks.

</details>

<details>
<summary><strong>Metrics</strong></summary>

- **`--telemetry-disabled`**: Disable connection to the Madara telemetry server.

- **`--telemetry-url <URL VERBOSITY>`**: The URL of the telemetry server with verbosity level.
  - [default: "wss://starknodes.com/submit 0"]

- **`--prometheus-port <PORT>`**: The port used by the Prometheus RPC service.
  - [default: 9615]

- **`--prometheus-external`**: Listen on all network interfaces for Prometheus.

- **`--prometheus-disabled`**: Disable the Prometheus service.

</details>

<details>
<summary><strong>P2P</strong></summary>

**Coming soon**

</details>

### Environment Variables

Set up your node's environment variables using the `STARKNET_` prefix. For example:

- `STARKNET_BASE_PATH=/path/to/data`
- `STARKNET_LOG=info`

These variables allow you to adjust the node's configuration without using command-line arguments.

### Configuration File

You can use a JSON, TOML, or YAML file to structure your configuration settings. Specify your configuration file on startup with the `-c` option. Here's a basic example in JSON format:

```json
{
  "name": "Deoxys",
  "base_path": "../deoxys-db",
  "network": "mainnet",
  "l1_endpoint": "your-ethereum-api-url",
  "rpc_port": 9944,
  "rpc_cors": "*",
  "rpc_external": true,
  "prometheus_external": true
}
```

> 💡 **Tip:** Review settings carefully for optimal performance and refer to Starknet's official documentation for detailed configuration guidelines.

Always test your configuration in a non-production environment before rolling it out to a live node to prevent downtime and other potential issues.

> ℹ️ **Note:** For a custom chain configuration, you can head to the configuration section of chain operators deployments.

## 📸 Snapshots

Snapshots are under development and will be available through the `--snap <BLOCK_NUMBER>` parameter.

## 🌐 Interactions

Madara fully supports all the JSON-RPC methods as specified in the Starknet mainnet official [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs). These methods can be categorized into three main types: Read-Only Access Methods, Trace Generation Methods, and Write Methods.

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

> ℹ️ **Info:** Madara currently supports the latest [JSON-RPC specs](https://github.com/starkware-libs/starknet-specs) up to version `v0.7.1`.

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

You can use any JSON-RPC client to interact with the Madara node, such as `curl`, `httpie`, or a custom client in your preferred programming language. For more detailed information and examples on each method, please refer to the [Starknet JSON-RPC specs](https://github.com/starkware-libs/starknet-specs).

> ⚠️ **Warning:** Write methods are forwarded to the Sequencer for execution. Ensure you handle errors appropriately as per the JSON-RPC schema.

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

We welcome contributions from the community! Please read our [contributing guidelines](CONTRIBUTING.md) to get started.

## 🤝 Partnerships

To establish a partnership with the Madara team, or if you have any suggestions or special requests, feel free to reach us on [Telegram](https://t.me/madara_alliance).

## ⚠️ License

Copyright (c) 2022-present.

Madara is open-source software licensed under the [Apache-2.0 License](https://github.com/madara-alliance/madara/blob/main/LICENSE).