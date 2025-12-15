# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in the orchestrator.

## Project Overview

The Madara Orchestrator is a critical service that runs alongside the Madara node to coordinate block processing, proof generation, data submission, and state transitions. It manages a multi-stage job processing pipeline with distributed workers and queue-based execution.

**Key capabilities:**

- Queue-based job management with three-phase execution (Creation → Processing → Verification)
- Distributed worker architecture for scaling job processing
- Multiple prover backends (SHARP, ATLANTIC)
- Multiple settlement layers (Ethereum, Starknet)
- Multiple DA layers (Ethereum, Starknet)
- Multi-layer support (L2, L3)
- Self-healing mechanism for orphaned jobs
- OpenTelemetry instrumentation for monitoring

## Common Commands

### Building

```bash
# Release build
cargo build --release

# Build with features
cargo build --release --features "ethereum,with_mongodb,with_sqs,starknet"

# Build SNOS (requires Python 3.9)
make snos
```

### Running

```bash
# Setup mode (initializes AWS infrastructure)
cargo run --release --bin orchestrator setup \
    --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge \
    --event-bridge-type rule

# Run mode (starts orchestrator)
RUST_LOG=info cargo run --release --bin orchestrator run \
    --sharp --aws --settle-on-ethereum \
    --aws-s3 --aws-sqs --aws-sns \
    --da-on-ethereum --mongodb
```

### Testing

```bash
# Unit/Integration tests with coverage
RUST_LOG=debug RUST_BACKTRACE=1 cargo llvm-cov nextest \
    --release \
    --features testing \
    --lcov \
    --output-path lcov.info \
    --test-threads=1 \
    --workspace \
    --exclude=e2e-tests \
    --no-fail-fast

# E2E test
RUST_LOG=info cargo test --features testing test_orchestrator_workflow -- --nocapture
```

### Mock Services

```bash
# Mock Atlantic server
cargo run --bin orchestrator run --mock-atlantic-server

# Mock prover service (Docker)
docker run -p 6000:6000 ocdbytes/mock-prover:latest
```

## Architecture Overview

### Crate Organization

**`crates/da-clients/`** - Data Availability Clients

- `da-client-interface/`: Trait defining DA operations (publish state diff, verify inclusion)
- `ethereum/`: Ethereum DA client implementation
- `starknet/`: Starknet DA client implementation

**`crates/prover-clients/`** - Proof Generation Clients

- `prover-client-interface/`: Trait for prover services (submit task, get status, get proof)
- `sharp-service/`: SHARP prover integration
- `atlantic-service/`: ATLANTIC prover integration
- `gps-fact-checker/`: GPS fact verification utilities

**`crates/settlement-clients/`** - Settlement Layer Clients

- `settlement-client-interface/`: Trait for settlement operations (register proof, update state)
- `ethereum/`: Ethereum settlement client
- `starknet/`: Starknet settlement client

**`crates/utils/`** - Shared Utilities

- Metrics collection and OpenTelemetry instrumentation
- HTTP client utilities
- Layer/environment utilities

### Source Modules

| Module                           | Purpose                                                                                 |
| -------------------------------- | --------------------------------------------------------------------------------------- |
| `cli/`                           | Command-line argument parsing (Run & Setup modes)                                       |
| `core/`                          | Core abstractions and traits                                                            |
| `core/client/`                   | Client interfaces (DatabaseClient, QueueClient, StorageClient, AlertClient, LockClient) |
| `core/config/`                   | Configuration management and cloud provider setup                                       |
| `worker/`                        | Job processing workers and event handlers                                               |
| `worker/event_handler/`          | Job-specific handlers (SNOS, Proving, DA, StateUpdate, ProofRegistration, Aggregator)   |
| `worker/event_handler/triggers/` | Workers that trigger job creation                                                       |
| `worker/controller/`             | WorkerController that manages all worker lifecycle                                      |
| `types/`                         | Domain types and data structures                                                        |
| `types/jobs/`                    | Job definitions, status enums, metadata                                                 |
| `types/batch/`                   | Batch and aggregator batch definitions                                                  |
| `compression/`                   | Blob compression utilities                                                              |
| `setup/`                         | Resource initialization and AWS setup                                                   |
| `server/`                        | HTTP API server (Axum-based)                                                            |
| `error/`                         | Error types and handling                                                                |

### Job Execution Pipeline

```
┌─────────────────────────────────────────────────────────┐
│                   Job Lifecycle                         │
├─────────────────────────────────────────────────────────┤
│ 1. JobTrigger.run_worker()                              │
│    - Checks for failed jobs (halts if found)            │
│    - Heals orphaned jobs (self-healing mechanism)       │
│    - Creates new jobs based on block availability       │
│                                                         │
│ 2. JobService.queue_job_for_processing()                │
│    - Adds job to processing queue                       │
│                                                         │
│ 3. EventWorker consumes from queue                      │
│    - Calls JobHandlerTrait.process_job()                │
│    - Updates job status to PendingVerification          │
│                                                         │
│ 4. EventWorker consumes from verification queue         │
│    - Calls JobHandlerTrait.verify_job()                 │
│    - Handles Success/Rejection/Timeout                  │
└─────────────────────────────────────────────────────────┘
```

### Job Types

| Job Type            | Purpose                            |
| ------------------- | ---------------------------------- |
| `SnosRun`           | Execute SNOS on a block            |
| `ProofCreation`     | Generate proof from SNOS output    |
| `ProofRegistration` | Register proof on settlement layer |
| `DataSubmission`    | Submit state diff to DA layer      |
| `StateTransition`   | Update state on settlement layer   |
| `Aggregator`        | Aggregate multiple proofs          |

### Job State Machine

```
Created → LockedForProcessing → PendingVerification → Completed
                ↓                       ↓
            VerificationFailed    VerificationTimeout
                ↓                       ↓
              Failed                 Failed
```

## Configuration

### Required Environment Variables

**AWS Configuration:**

```
AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION
AWS_ENDPOINT_URL (for localstack)
```

**Database (MongoDB):**

```
MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL
MADARA_ORCHESTRATOR_DATABASE_NAME
```

**Prover Services:**

```
SHARP: MADARA_ORCHESTRATOR_SHARP_URL, MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID
ATLANTIC: MADARA_ORCHESTRATOR_ATLANTIC_API_KEY, MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL
```

**Atlantic API Documentation:**

- Swagger UI: https://atlantic.api.herodotus.cloud/docs/
- OpenAPI JSON: https://atlantic.api.herodotus.cloud/docs/json

**Settlement Layers:**

```
Ethereum: MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL, MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY
Starknet: MADARA_ORCHESTRATOR_STARKNET_SETTLEMENT_RPC_URL, MADARA_ORCHESTRATOR_STARKNET_PRIVATE_KEY
```

**Batching:**

```
MADARA_ORCHESTRATOR_MAX_BATCH_TIME_SECONDS
MADARA_ORCHESTRATOR_MAX_BATCH_SIZE
MADARA_ORCHESTRATOR_MAX_NUM_BLOBS
```

### CLI Argument Structure

- Provider group: `--aws` (required)
- Storage group: `--aws-s3` (requires provider)
- Queue group: `--aws-sqs` (requires provider)
- Alert group: `--aws-sns` (requires provider)
- Prover group: `--sharp` or `--atlantic` (mutually exclusive)
- Settlement group: `--settle-on-ethereum` or `--settle-on-starknet`
- DA group: `--da-on-ethereum` or `--da-on-starknet`

## Common Patterns

### Trait-Based Abstraction

- `DatabaseClient`: Database operations
- `QueueClient`: Message queue operations
- `StorageClient`: File storage (S3)
- `AlertClient`: Notifications (SNS)
- `ProverClient`: Proof generation
- `SettlementClient`: Settlement layer interactions
- `DaClient`: Data availability layer
- `LockClient`: Distributed locking
- `JobHandlerTrait`: Job-specific processing logic
- `JobTrigger`: Worker trigger logic

### Error Handling

- Custom error enums per module (e.g., `JobError`, `QueueError`, `DatabaseError`)
- `thiserror` for error definition
- Comprehensive error context in logs

### Testing Patterns

- `mockall` for automocking traits
- `mockall_double` for dependency injection in tests
- `rstest` for parameterized tests
- Feature flags (`testing`, `with_mongodb`, `with_sqs`, `ethereum`, `starknet`)

## Important Implementation Notes

### When Working with Jobs

- Jobs follow the three-phase lifecycle: Creation → Processing → Verification
- Use `JobHandlerTrait` for job-specific logic
- Always implement `max_process_attempts()` and `max_verification_attempts()`
- Orphaned jobs (stuck in LockedForProcessing) are automatically healed

### When Working with Workers

- Workers are managed by `WorkerController`
- Each queue type has its own `EventWorker`
- Use cancellation tokens for graceful shutdown
- Different queue sets for L2 vs L3

### When Adding New Job Types

1. Add variant to `JobType` enum in `types/jobs/types.rs`
2. Implement `JobHandlerTrait` for the new job type
3. Add corresponding queue types in `types/queue.rs`
4. Register handler in `worker/event_handler/factory.rs`
5. Implement trigger logic if job creation is automatic

### When Working with Batches

- `SnosBatch`: Groups blocks for SNOS processing
- `AggregatorBatch`: Groups SNOS batches for proof aggregation
- Use hierarchical relationship with closure semantics

## Build Features

- `ethereum`: Ethereum DA client
- `starknet`: Starknet DA client
- `with_mongodb`: MongoDB support
- `with_sqs`: AWS SQS support
- `testing`: Testing features (mock servers, test utilities)

## Dependencies

**Key Framework Dependencies:**

- `tokio`: Async runtime
- `axum`: Web framework
- `serde/serde_json`: Serialization
- `tracing`: Logging and instrumentation
- `opentelemetry`: Observability

**AWS Integration:**

- `aws-sdk-sqs`, `aws-sdk-s3`, `aws-sdk-sns`, `aws-sdk-eventbridge`
- `omniqueue`: Queue abstraction

**Blockchain:**

- `starknet`: Starknet client
- `alloy`: Ethereum client
- `cairo-vm`: Cairo execution

**Database:**

- `mongodb`: NoSQL database
