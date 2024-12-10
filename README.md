# Madara Orchestrator 🎭

The Madara orchestrator is designed to be an additional service which runs in
parallel to Madara and handles various critical jobs that ensure proper block
processing, proof generation, data submission and state transitions.

## 📋 Overview

The Madara Orchestrator coordinates and triggers five primary jobs in sequence,
managing their execution through a centralized queue system, alowing
for multiple orchestrator to run together!

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

### Prerequisites

1. **Madara Node**

   - Required for block processing
   - Follow setup instructions at [Madara Documentation](https://github.com/madara-alliance/madara)

2. **Prover Service**

   - ATLANTIC must be running

3. **Infrastructure Dependencies**
   - MongoDB for job management
   - AWS services (or Localstack for local development):
     - SQS for queues
     - S3 for data storage
     - SNS for alerts
     - EventBridge for scheduling

## 🚀 Installation & Setup

### Setup Mode

Setup mode configures the required AWS services and dependencies.
Use the following command:

```bash
cargo run --release --bin orchestrator setup --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge
```

Note: Setup mode is currently in development. A fresh setup is required
if the process fails mid-way.

### Run Mode

Run mode executes the orchestrator's job processing workflow. Example command:

```bash
RUST_LOG=info cargo run --release --bin orchestrator run --atlantic --aws --settle-on-ethereum --aws-s3 --aws-sqs --aws-sns --da-on-ethereum --mongodb
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

9. **Scheduling**:

   - `--aws-event-bridge`: Cron job scheduling

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

Note: These configurations are also picked up from

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

## 🧪 Testing

The Madara Orchestrator supports three types of tests:

### Types of Tests

1. **E2E Tests** 🔄

   - End-to-end workflow testing
   - Tests orchestrator functionality on block 66645 of Starknet
   - Uses mocked proving endpoints

2. **Integration & Unit Tests** 🔌

### Running Tests

#### Required Services

- MongoDB running on port 27017
- Localstack running on port 4566
- Anvil (local Ethereum node)

#### Environment Configuration

```bash
export MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL=<ethereum-rpc-url>
export MADARA_ORCHESTRATOR_RPC_FOR_SNOS=<snos-rpc-url>
export AWS_REGION=us-east-1
```

#### Running E2E Tests

```bash
RUST_LOG=info cargo test --features testing test_orchestrator_workflow -- --nocapture
```

#### Running Integration and Unit Tests

The orchestrator uses LLVM coverage testing to ensure comprehensive test coverage
of the codebase.

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
- Enables debug logging and full backtraces for better error diagnosis

The coverage report (`lcov.info`) can be used with various code coverage
visualization tools.

## 📓 More Information

- Read the architecture present at `./docs/orchestrator_da_sequencer_diagram.png`
