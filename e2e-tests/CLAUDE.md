# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with the e2e-tests (orchestrator workflow tests).

## Project Overview

The e2e-tests folder contains orchestrator-specific workflow testing focused on proving,
aggregation, and state transitions. It validates the complete orchestrator job pipeline
with heavily mocked external services.

**Key capabilities:**

- Tests complete orchestrator workflow (SNOS → Proving → Aggregation → StateTransition)
- Heavily mocked external services (SHARP, Starknet)
- Database state validation via polling
- Fresh setup per test

**Related:** See also `../e2e/` for full-stack bridge integration testing.

## Common Commands

### Running Tests

```bash
# Run orchestrator workflow test
cargo test --package e2e-tests --test test_orchestrator_workflow -- --nocapture

# With debug logging
RUST_LOG=debug cargo test --package e2e-tests test_orchestrator_workflow -- --nocapture
```

### Prerequisites

- `.env.test` file in working directory
- Docker running (for MongoDB)
- Anvil installed

## Architecture Overview

### Directory Structure

```text
e2e-tests/
├── src/
│   ├── lib.rs              # Module exports and utilities
│   ├── node.rs             # Orchestrator process management
│   ├── anvil.rs            # Anvil setup and contract deployment
│   ├── mongodb.rs          # MongoDB wrapper
│   ├── sharp.rs            # SHARP client with mocking
│   ├── starknet_client.rs  # Starknet RPC client with mocking
│   ├── mock_server.rs      # HTTP mock server utilities
│   └── utils.rs            # Helper functions
├── tests.rs                # Main test file
└── artifacts/              # Test data (state updates, nonces, ABIs)
```

### Services

| Service        | Type      | Purpose                |
| -------------- | --------- | ---------------------- |
| MongoDB        | Container | Job persistence        |
| Anvil          | Process   | L1 settlement          |
| StarknetClient | Mocked    | Starknet RPC responses |
| SharpClient    | Mocked    | SHARP prover responses |
| Orchestrator   | Process   | Job processing         |

### Test Flow

```text
1. Initialize MongoDB (fresh instance)
2. Initialize StarknetClient (mock)
3. Initialize SharpClient (mock)
4. Initialize Anvil + deploy contracts
5. Insert SNOS job into database
6. Insert Proving jobs for all blocks
7. Mock SHARP endpoints
8. Queue SNOS job via SQS
9. Start orchestrator in run mode
10. Poll DB for state transitions:
    - Batch creation
    - SNOS completion
    - ProofCreation completion
    - Aggregator completion
    - StateTransition completion
```

### Job State Validation

Each job type is polled with specific timeouts:

- Polling interval: 5 seconds
- Maximum timeout: 1500-2400 seconds per job type
- Version tracking for job updates

## Configuration

### Environment Variables (`.env.test`)

```text
MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL=mongodb://localhost:27017
MADARA_ORCHESTRATOR_DATABASE_NAME=test_db
MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL=http://localhost:8545
MADARA_ORCHESTRATOR_SHARP_URL=http://localhost:3001
MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS=0x...
MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS=0x...
MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS=0x...
MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS=525593
MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS=525593
```

### Test Data

Located in `artifacts/`:

- `get_state_update_<block>.json`: State update for block
- `nonces_<block>.json`: Nonces for block
- Contract ABIs and deployment data

## Mock Endpoints

### SHARP Client Mocking

```text
POST /add_job → {code: JOB_RECEIVED_SUCCESSFULLY}
GET /get_status → {status: ONCHAIN, validation_done: true}
POST /create_bucket → {code: BUCKET_CREATED_SUCCESSFULLY, bucket_id: ...}
POST /close_bucket → {code: BUCKET_CLOSED_SUCCESSFULLY}
GET /aggregator_task_id → {task_id: <uuid>}
```

### Starknet Client Mocking

```text
starknet_getNonce → Nonce from JSON artifact
starknet_getStateUpdate → State update from JSON artifact
starknet_blockNumber → L2 block number
```

## Important Implementation Notes

### When Modifying Tests

- Test uses hardcoded block number "525593" via rstest parameter
- Job assertions use polling with 5-second intervals
- All external services are mocked for isolation

### When Adding Mock Endpoints

1. Add mock setup in test function
2. Use `httpmock` for HTTP mocking
3. Store test data in `artifacts/`

### When Debugging Failures

- Check job state in MongoDB
- Verify mock endpoints are responding correctly
- Check orchestrator logs for error messages
- Ensure all environment variables are set

## Comparison with e2e

| Aspect            | e2e                  | e2e-tests               |
| ----------------- | -------------------- | ----------------------- |
| **Focus**         | Bridge transactions  | Orchestrator pipeline   |
| **Services**      | 13 (full stack)      | 4 (minimal + mocks)     |
| **DB Strategy**   | Persistent snapshots | Fresh per test          |
| **Mocking**       | Minimal (real L1)    | Heavy (SHARP, Starknet) |
| **Assertions**    | Transaction finality | DB state polling        |
| **Test Duration** | ~30+ mins            | ~40+ mins               |
