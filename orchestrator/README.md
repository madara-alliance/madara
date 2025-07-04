# Madara Orchestrator 🎭

The Madara orchestrator is designed to be an additional service which runs in
parallel to Madara and handles various critical jobs that ensure proper block
processing, proof generation, data submission and state transitions.

> 📝 **Note**: These instructions are verified for Ubuntu systems with AMD64 architecture. While most steps remain similar
> for macOS, some package names and installation commands may differ.

## Table of Contents

- [Overview](#-overview)
- [Architecture](#️-architecture)
  - [Job Processing Model](#job-processing-model)
  - [Queue Structure](#queue-structure)
  - [Workflow](#workflow)
- [Technical Requirements](#️-technical-requirements)
  - [System Dependencies](#system-dependencies)
  - [Core Dependencies](#core-dependencies)
- [Installation & Setup](#-installation--setup)
  - [Building from Source](#building-from-source)
  - [Local Development Setup](#local-development-setup)
  - [Setup Mode](#setup-mode)
  - [Run Mode](#run-mode)
  - [Command Line Options](#command-line-options)
- [Configuration](#️-configuration)
  - [AWS Configuration](#aws-configuration)
  - [Prover Configuration](#prover-configuration)
  - [Database Configuration](#database-configuration)
- [Testing](#-testing)
  - [Local Environment Setup](#local-environment-setup)
  - [Types of Tests](#types-of-tests)
  - [Running Tests](#running-tests)
- [Monitoring](#-monitoring)
- [Error Handling](#-error-handling)
- [Additional Resources](#additional-resources)

## 📋 Overview

The Madara Orchestrator coordinates and triggers five primary jobs in sequence,
managing their execution through a centralized queue system, allowing
for multiple orchestrators to run together!

1. **SNOS (Starknet OS) Job** 🔄

   - Identifies blocks that need processing.
   - Triggers SNOS run on identified blocks.
   - Tracks SNOS execution status and PIE (Program Independent Execution) generation

2. **Proving Job** ✅

   - Coordinates proof generation by submitting PIE to proving services
   - Tracks proof generation progress and completion

3. **Data Submission Job** 📤

   - Manages state update data preparation for availability layers
   - If needed, coordinates blob submission to data availability layers
   - Currently integrates with Ethereum (EIP-4844 blob transactions)
   - Additional DA layer integrations in development (e.g., Celestia)

4. **State Transition Job** 🔄
   - Coordinates state transitions with settlement layers
   - Manages proof and state update submissions
   - Handles integration with Ethereum and Starknet settlement layers

Each job is managed through a queue-based system where the orchestrator:

- Determines when and which blocks need processing
- Triggers the appropriate services
- Monitors job progress
- Manages job dependencies and sequencing
- Handles retries and failure cases

## 🏛️ Architecture

### Job Processing Model

The orchestrator implements a queue-based architecture where each job type
follows a three-phase execution model:

1. **Creation**: Jobs are spawned based on block availability
2. **Processing**: Core job logic execution
3. **Verification**: Result validation and confirmation

### Queue Structure

The system uses dedicated queues for managing different job phases:

- Worker Trigger Queue
- SNOS Processing/Verification Queues
- Proving Processing/Verification Queues
- Data Submission Processing/Verification Queues
- State Update Processing/Verification Queues
- Job Failure Handling Queue

### Workflow

1. Cron jobs trigger worker tasks via the worker-trigger queue
2. Workers determine block-level job requirements
3. Jobs are created and added to processing queues
4. Processed jobs move to verification queues
5. Verified jobs are marked as complete in the database

## 🛠️ Technical Requirements

### System Dependencies

> For macOS users, use `brew install` instead of `apt install` for these dependencies.

- Build essentials (`build-essential`)
- OpenSSL (`libssl-dev`)
- Package config (`pkg-config`)
- Python 3.9 with development files
- GMP library (`libgmp-dev`)

### Core Dependencies

- [Git](https://git-scm.com/book/en/v2/Getting-Started-Installing-Git)
- [Rust](https://www.rust-lang.org/tools/install)
- [Madara Node](https://github.com/madara-alliance/madara)
  - Required for block processing
  - Follow setup instructions at [Madara Documentation](https://github.com/madara-alliance/madara)
- Prover Service (ATLANTIC)
- MongoDB for job management
- AWS services (or Localstack for local development):
  - SQS for queues
  - S3 for data storage
  - SNS for alerts
  - EventBridge for scheduling

> 🚨 **Important Note**: SNOS requires the `get_storage_proof` RPC endpoint to function.
> Currently, this endpoint is not implemented in Madara.
>
> 🚧 Until madara implements the `get_storage_proof` endpoint, you need to run Pathfinder alongside Madara:
>
> - Madara will run in sequencer mode
> - Pathfinder will sync with Madara
> - The orchestrator will use Pathfinder's RPC URL for SNOS and state update fetching
>
> This setup is temporary until either:
>
> 1. SNOS is adapted to work without the `get_storage_proof` endpoint, or
> 2. The `get_storage_proof` endpoint is implemented in Madara

## 🚀 Installation & Setup

### Building from Source

1. **Install System Dependencies**

   ```bash
   # Ubuntu/Debian
   sudo apt-get update
   sudo apt install build-essential openssl pkg-config libssl-dev
   sudo apt install python3.9 python3.9-venv python3.9-distutils libgmp-dev python3.9-dev

   # For macOS
   brew install openssl pkg-config gmp python@3.9
   ```

   > 🚨 **Note**: python 3.9 is required for the `SNOS` to create `os_latest.json` hence the `python3.9` in the above command.

2. **Install Rust** (Cross-platform)

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source ~/.bashrc  # Or source ~/.zshrc for macOS
   ```

3. **Clone Repository**

   ```bash
   git clone https://github.com/madara-alliance/madara-orchestrator.git
   cd madara-orchestrator
   git submodule update --init
   ```

4. **Build SNOS**

   ```bash
   make snos
   ```

   > 🚨 **Note**: python 3.9 is required for the `SNOS` to create `os_latest.json`

5. **Build Project**

   ```bash
   cargo build --release
   ```

### Local Development Setup

1. **Install Docker** (Cross-platform)
   Follow the official installation guides:

   - [Ubuntu Installation Guide](https://docs.docker.com/engine/install/ubuntu/#install-using-the-repository)
   - [macOS Installation Guide](https://docs.docker.com/desktop/install/mac-install/)

2. **Install Foundry** (Cross-platform)

   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

3. **Start Local Services**

   ```bash
   # Start MongoDB
   docker run -d -p 27017:27017 mongo

   # Start Localstack
   docker run -d -p 4566:4566 localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314

   # Start Anvil in a separate terminal
   anvil --block-time 1
   ```

4. **Setup Mock Proving Service**

   🚧 This setup is for development purposes only:

   ```bash
   # Start the mock prover service using Docker
   docker run -d -p 6000:6000 ocdbytes/mock-prover:latest

   # Set the mock prover URL in your .env
   MADARA_ORCHESTRATOR_SHARP_URL=http://localhost:6000
   ```

5. **Run Pathfinder** (Choose one method)

   > 🚨 **Important Note**:
   >
   > - Pathfinder requires a WebSocket Ethereum endpoint (`ethereum.url`). Since Anvil doesn't support WebSocket yet,
   >   you'll need to provide a different Ethereum endpoint (e.g., Alchemy, Infura). This is okay for local development
   >   as Pathfinder only uses this to get the state update from core contract.
   > - Make sure `chain-id` matches your Madara chain ID (default: `MADARA_DEVNET`)
   > - The `gateway-url` and `feeder-gateway-url` should point to your local Madara node (default: `http://localhost:8080`)

   a. **From Source** (Recommended for development)

   ```bash
   # Clone the repository
   git clone https://github.com/eqlabs/pathfinder.git
   cd pathfinder

   # Run pathfinder
   cargo run --bin pathfinder -- \
       --network custom \
       --chain-id MADARA_DEVNET \
       --ethereum.url wss://eth-sepolia.g.alchemy.com/v2/xxx \  # Replace with your Ethereum endpoint
       --gateway-url http://localhost:8080/gateway \
       --feeder-gateway-url http://localhost:8080/feeder_gateway \
       --storage.state-tries archive \
       --data-directory ~/Desktop/pathfinder_db/ \
       --http-rpc 127.0.0.1:9545
   ```

   b. **Using Docker**

   ```bash
   # Create data directory
   mkdir -p ~/pathfinder_data

   # Run pathfinder container
   docker run \
       --name pathfinder \
       --restart unless-stopped \
       -p 9545:9545 \
       --user "$(id -u):$(id -g)" \
       -e RUST_LOG=info \
       -v ~/pathfinder_data:/usr/share/pathfinder/data \
       eqlabs/pathfinder \
       --network custom \
       --chain-id MADARA_DEVNET \
       --ethereum.url wss://eth-sepolia.g.alchemy.com/v2/xxx \  # Replace with your Ethereum endpoint
       --gateway-url http://localhost:8080/gateway \
       --feeder-gateway-url http://localhost:8080/feeder_gateway \
       --storage.state-tries archive
   ```

6. **Deploy Mock Verifier Contract**

   🚧 For development purposes, you can deploy the mock verifier contract using:

   ```bash
   ./scripts/dummy_contract_deployment.sh http://localhost:9944 0
   ```

   This script:

   - Takes the Madara endpoint and block number as parameters
   - Automatically deploys both the verifier contract and core contract
   - Sets up the necessary contract relationships
   - The deployed contract addresses will be output to the console

   ```bash
   MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS=<deployed-core-contract-address>
   MADARA_ORCHESTRATOR_VERIFIER_ADDRESS=<deployed-verifier-address>
   ```

🚧 Note: The mock services are intended for development and testing purposes only.
In production, you'll need to use actual proving services and verifier contracts.

### Setup Mode

Setup mode configures the required AWS services and dependencies.
Use the following command:

```bash
cargo run --release --bin orchestrator setup --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type rule
```

> 🚨 **Note**:
>
> - Setup mode is currently in development. A fresh setup is required
>   if the process fails mid-way.
> - The `event-bridge-type` needs to be `rule` in case of localstack.
> - The `event-bridge-type` should be `schedule` in case of AWS.

### Run Mode

Run mode executes the orchestrator's job processing workflow. Example command:

```bash
RUST_LOG=info cargo run --release --bin orchestrator run \
    --sharp \
    --aws \
    --settle-on-ethereum \
    --aws-s3 \
    --aws-sqs \
    --aws-sns \
    --da-on-ethereum \
    --mongodb
```

### Command Line Options

1. **Prover Services** (choose one):

   - `--atlantic`: Use Atlantic prover
   - `--sharp`: Use SHARP prover

2. **Settlement Layer** (choose one):

   - `--settle-on-ethereum`: Use Ethereum
   - `--settle-on-starknet`: Use Starknet

3. **Data Availability**:

   - `--da-on-ethereum`: Use Ethereum

4. **Infrastructure**:

   - `--aws`: Use AWS services (or Localstack)

5. **Data Storage**:

   - `--aws-s3`: Store state updates and program outputs

6. **Database**:

   - `--mongodb`: Store job information

7. **Queue System**:

   - `--aws-sqs`: Message queue service

8. **Alerting**:

   - `--aws-sns`: Notification service

9. **Event Bridge Scheduling**:

   - `--aws-event-bridge`: Enable AWS Event Bridge
   - `--event-bridge-type`: Specify the type of Event Bridge (rule or schedule)

10. **Monitoring**:
    - `--otel-service-name`: OpenTelemetry service name
    - `--otel-collector-endpoint`: OpenTelemetry collector endpoint

## ⚙️ Configuration

The orchestrator uses environment variables for configuration.
Create a `.env` file with the following sections:

### AWS Configuration

```env
AWS_ACCESS_KEY_ID=<your-key>
AWS_SECRET_ACCESS_KEY=<your-secret>
AWS_REGION=<your-region>
```

Note: These configurations are also picked up from your AWS credentials file (~/.aws/credentials)
or environment variables if not specified in the .env file.

### Prover Configuration

```env
# SHARP Configuration
MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID=<customer-id>
MADARA_ORCHESTRATOR_SHARP_URL=<sharp-url>
# or
# ATLANTIC Configuration
MADARA_ORCHESTRATOR_ATLANTIC_API_KEY=<api-key>
MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL=<service-url>
```

### Database Configuration

```env
MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL=mongodb://localhost:27017
MADARA_ORCHESTRATOR_DATABASE_NAME=orchestrator
```

For a complete list of configuration options, refer to the `.env.example` file
in the repository.

## 🔍 Monitoring

The orchestrator includes a telemetry system that tracks:

- Job execution metrics
- Processing time statistics
- RPC performance metrics

OpenTelemetry integration is available for detailed monitoring.
It requires a `Otel-collector` url to be able to send metrics/logs/traces.

## 🐛 Error Handling

- Failed jobs are moved to a dedicated failure handling queue
- Automatic retry mechanism with configurable intervals
- Failed jobs are tracked in the database for manual inspection after maximum retries
- Integrated telemetry system for monitoring job failures

## 📓 Testing

### Local Environment Setup

🚧 This setup is for development purposes. For production deployment, please refer to our deployment documentation.

Before running tests, ensure you have:

1. **Required Services Running**:

   - MongoDB on port 27017
   - Localstack on port 4566
   - Anvil (local Ethereum node)

2. **Environment Configuration**:

   ```bash
   export MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL=<ethereum-rpc-url>
   export MADARA_ORCHESTRATOR_RPC_FOR_SNOS=<snos-rpc-url>
   export AWS_REGION=us-east-1
   ```

### Types of Tests

1. **E2E Tests** 🔄

   🚧 Development test environment:

   - End-to-end workflow testing
   - Tests orchestrator functionality on block 66645 of Starknet
   - Uses mocked proving endpoints

2. **Integration & Unit Tests** 🔌
   - Tests component interactions
   - Verifies individual functionalities

### Running Tests

#### Running E2E Tests

```bash
RUST_LOG=info cargo test --features testing test_orchestrator_workflow -- --nocapture
```

#### Running Integration and Unit Tests

```bash
RUST_LOG=debug RUST_BACKTRACE=1 cargo llvm-cov nextest \
    --release \
    --features testing \
    --lcov \
    --output-path lcov.info \
    --test-threads=1 \
    --workspace \
    --exclude=e2e-tests \
    --no-fail-fast
```

This command:

- Generates detailed coverage reports in LCOV format
- Excludes E2E tests from coverage analysis
- Runs tests sequentially (single thread)
- Continues testing even if failures occur
- Enables debug logging and full backtraces for better error
  diagnosis

The coverage report (`lcov.info`) can be used with various code coverage
visualization tools.

## 📓 More Information

- Read the architecture present at `./docs/orchestrator_da_sequencer_diagram.png`

## Additional Resources

- Architecture Diagram: See `./docs/orchestrator_da_sequencer_diagram.png`
- [Madara Documentation](https://github.com/madara-alliance/madara)
- [LocalStack Documentation](https://docs.localstack.cloud/)
- [Foundry Documentation](https://book.getfoundry.sh/)
