# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with the e2e test framework.

## Project Overview

The e2e folder contains a comprehensive test framework library for testing the Madara Starknet sequencer system with multiple services working together. It tests full-stack integration for Layer 2 (L2) blockchain operations including deposits, withdrawals, and bridge transactions.

**Key capabilities:**
- Full-stack integration testing for L1-L2 bridge operations
- Multi-service orchestration (13+ services)
- Layered architecture with clear separation of concerns
- Fixture-based test setup with rstest

**Related:** See also `../e2e-tests/` for orchestrator-specific workflow testing.

## Common Commands

### Running Tests
```bash
# Full workflow validation
cargo test --package e2e --lib -- --nocapture --test-threads=1

# Specific bridge test
cargo test --package e2e test_bridge_deposit_and_withdraw -- --nocapture

# With debug logging
RUST_LOG=debug cargo test --package e2e -- --nocapture
```

### Prerequisites
- Docker running
- `anvil` installed (from Foundry)
- `forge` installed (from Foundry)
- AWS CLI configured (for LocalStack)
- `.env.e2e` file in working directory

## Architecture Overview

### Directory Structure

```
e2e/
├── src/
│   ├── setup/              # Environment initialization
│   │   ├── mod.rs          # ChainSetup facade
│   │   ├── config.rs       # Configuration and timeouts
│   │   ├── service_management.rs    # Service startup/shutdown
│   │   ├── dependency_validation.rs # Docker, Anvil, Forge checks
│   │   └── lifecycle_management.rs  # Graceful shutdown
│   ├── services/           # Service implementations (13+)
│   │   ├── anvil/          # L1 Ethereum chain
│   │   ├── madara/         # L2 Starknet sequencer
│   │   ├── pathfinder/     # Starknet full node
│   │   ├── mongodb/        # Orchestrator database
│   │   ├── localstack/     # AWS services mock
│   │   ├── orchestrator/   # Proof pipeline
│   │   ├── bootstrapper/   # Contract initialization
│   │   └── ...
│   └── tests/              # Test implementations
│       ├── setup.rs        # rstest fixture
│       └── deposit_withdraw/  # Bridge tests
└── config/
    ├── madara.yaml         # Madara chain configuration
    └── bootstrapper.json   # Contract addresses
```

### Service Dependencies

```
Infrastructure (MongoDB, LocalStack)
         ↓
L1 Setup (Anvil → Mock Verifier Deployer → Bootstrapper)
         ↓
L2 Setup (Madara → Bootstrapper)
         ↓
Full Node Syncing (Pathfinder catches Madara)
         ↓
Orchestration (Orchestrator Setup → Runtime)
```

### Services Started

| Service | Purpose | Port |
|---------|---------|------|
| Anvil | L1 Ethereum chain | 8545 |
| MongoDB | Orchestrator database | 27017 |
| LocalStack | AWS S3, SQS, SNS, EventBridge | 4566 |
| Madara | L2 Starknet sequencer | 9944 (RPC), 9943 (Admin), 8080 (Gateway) |
| Pathfinder | Starknet full node | 9545 |
| Bootstrapper | L1/L2 contract initialization | N/A |
| Orchestrator | Proof pipeline coordinator | N/A |
| Mock Prover | STARK proof simulator | 3001 |

### Configuration Timeouts

```rust
validate_dependencies: 60s
start_infrastructure_services: 180s
setup_localstack_infrastructure: 180s
setup_mongodb_infrastructure: 180s
complete_l1_setup: 360s
complete_l2_setup: 1800s (30 min)
complete_full_node_syncing: 300s
start_mock_prover: 180s
complete_orchestration: 2400s (40 min)
```

## Test Framework

### Test Pattern
```rust
#[rstest]
#[tokio::test]
async fn test_bridge_deposit_and_withdraw(#[future] setup_chain: ChainSetup) {
    let chain = setup_chain.await;
    // Test logic using chain.services...
}
```

### Fixture Flow
1. Load `.env.e2e`
2. Create `ChainSetup` via `SetupConfigBuilder`
3. Validate dependencies (Docker, Anvil, Forge)
4. Start infrastructure (MongoDB, LocalStack in parallel)
5. Set up L1 (Anvil) and L2 (Madara)
6. Sync full node (Pathfinder)
7. Run test
8. Clean up via `drop()` handler

### Test Utilities
Located in `src/tests/deposit_withdraw/utils.rs`:
- `setup_l1_context()`: L1 Ethereum configuration
- `setup_l2_context()`: L2 Starknet/Madara configuration
- `wait_for_transactions_finality()`: Block until L2 txns finalize on L1

### Test Accounts
```rust
L2_ACCOUNT: 0x4fe5eea46caa0a1f344fafce82b39d66b552f00d3cd12e89073ef4b4ab37860
L1_ACCOUNT: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
```

## Important Implementation Notes

### When Adding New Tests
1. Use `#[rstest]` fixture for `ChainSetup`
2. Use `#[tokio::test]` for async tests
3. Add test utilities in `tests/` module
4. Consider adding new services if needed

### When Adding New Services
1. Create service module in `services/`
2. Implement start/stop logic
3. Add to service dependency graph in `service_management.rs`
4. Update shutdown order in `lifecycle_management.rs`

### When Modifying Configuration
- Timeouts are in `setup/config.rs`
- Service ports in `services/constants.rs`
- Chain config in `config/madara.yaml`
- Contract addresses in `config/bootstrapper.json`

---

# E2E-Tests (Orchestrator Workflow)

Located in `../e2e-tests/`, this tests the orchestrator's job processing pipeline with heavily mocked external services.

## E2E-Tests Commands

```bash
# Run orchestrator workflow test
cargo test --package e2e-tests --test test_orchestrator_workflow -- --nocapture

# With debug logging
RUST_LOG=debug cargo test --package e2e-tests test_orchestrator_workflow -- --nocapture
```

## E2E-Tests Architecture

**Focus:** Orchestrator job pipeline (SNOS → Proving → Aggregation → StateTransition)

**Services:**
- MongoDB (fresh per test)
- Anvil + contract deployment
- StarknetClient (mocked)
- SharpClient (mocked)
- Orchestrator

**Test Flow:**
1. Initialize MongoDB, StarknetClient (mock), SharpClient (mock), Anvil
2. Insert SNOS job into database
3. Insert Proving jobs for all blocks
4. Mock SHARP endpoints (add_job, close_bucket, create_bucket, aggregator_task_id)
5. Queue SNOS job via SQS
6. Start orchestrator in run mode
7. Poll DB for job state transitions (with timeouts)
8. Validate: Batch creation → SNOS completion → ProofCreation → Aggregator → StateTransition

**Configuration:** Via `.env.test` file with environment variables:
```
MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL
MADARA_ORCHESTRATOR_DATABASE_NAME
MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL
MADARA_ORCHESTRATOR_SHARP_URL
MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS
MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS
```

**Test Data:** Stored in `e2e-tests/artifacts/` (state updates, nonces, contract ABIs)

## Comparison

| Aspect | e2e | e2e-tests |
|--------|-----|-----------|
| **Focus** | Bridge transactions | Orchestrator pipeline |
| **Services** | 13 (full stack) | 4 (minimal + mocks) |
| **DB Strategy** | Persistent snapshots | Fresh per test |
| **Mocking** | Minimal (real L1) | Heavy (SHARP, Starknet) |
| **Test Duration** | ~30+ mins | ~40+ mins |
