# Mock Atlantic Server

A mock HTTP server that implements the Atlantic API endpoints the orchestrator uses for testing and development purposes.

## Overview

The Mock Atlantic Server provides the subset of the Atlantic HTTP interface used by the orchestrator, allowing you to test your
applications without depending on the external Atlantic service. It can simulate
various scenarios including failures and processing delays.

## Features

- 🚀 **Orchestrator API Compatibility**: Implements the Atlantic endpoints used by the orchestrator
- 🎭 **Configurable Behavior**: Simulate failures, processing delays, and different response patterns
- 🔄 **Realistic Job Lifecycle**: Jobs progress through realistic status transitions
- 📊 **Configurable Failure Rates**: Test error handling with controlled failure simulation
- 🏥 **Health Checks**: Built-in health check endpoint for monitoring
- 🧪 **Testing Support**: Easy integration for unit and integration tests

## How to Spin the Server

- Run with default settings (port 4001, no failures):

  ```bash
  cargo run -p utils-mock-atlantic-server -- --port 4001
  ```

- Run on port 8080:

  ```bash
  cargo run -p utils-mock-atlantic-server -- --port 8080
  ```

- Run on port 8080 with 10% failure rate:

  ```bash
  cargo run -p utils-mock-atlantic-server -- --port 8080 --failure-rate 0.1
  ```

## API Endpoints

### Job Management

- `POST /atlantic-query?apiKey={key}` - Submit a new proving job
- `GET /atlantic-query/{job_id}` - Get job status and details

### Health & Monitoring

- `GET /is-alive` - Health check endpoint

### Documentation

For the upstream Atlantic API surface, see the [swagger.json](./swagger.json) file which contains the OpenAPI 3.0.3
specification from the Herodotus Atlantic API. The mock server implements only the endpoints listed above.

## Usage

### Running as a Standalone Server

```bash
# Run with default settings (port 4001, no failures)
cargo run -p utils-mock-atlantic-server -- --port 4001

# Run on custom port
cargo run -p utils-mock-atlantic-server -- --port 8080

# Run with failure simulation (10% failure rate)
cargo run -p utils-mock-atlantic-server -- --port 8080 --failure-rate 0.1
```

### Using with Atlantic Client

To use the mock server with the real Atlantic client, point the `atlantic_service_url` to your mock server when
the mock fact hash mode is enabled.

### Using with Orchestrator

The orchestrator can start this mock server for local testing. When you pass
`--mock-atlantic-server` with `--prover atlantic` and `--atlantic-network TESTNET`,
the orchestrator starts the mock server on port 4001. Set
`--atlantic-service-url http://127.0.0.1:4001` so the Atlantic client sends
requests to it. `--atlantic-mock-fact-hash true` tells Atlantic to use mock fact
hash mode.

This setup lets you:

1. Start the mock Atlantic server from the orchestrator process
2. Route Atlantic client requests to the local mock server
3. Disable fact checking for faster testing

Besides the normal `orchestrator run` arguments, provide the required Atlantic
arguments. This snippet shows the Atlantic-specific part:

```bash
cargo run --release --bin orchestrator run \
  --prover atlantic \
  --mock-atlantic-server \
  --atlantic-api-key "mock-key" \
  --atlantic-service-url "http://127.0.0.1:4001" \
  --atlantic-rpc-node-url "http://127.0.0.1:9944/rpc/v0_10_2" \
  --atlantic-mock-fact-hash "true" \
  --atlantic-prover-type "ethereum" \
  --atlantic-settlement-layer "ethereum" \
  --atlantic-verifier-contract-address "0x0000000000000000000000000000000000000000" \
  --atlantic-network "TESTNET"
```

When `--mock-atlantic-server` starts the embedded server, the orchestrator will log:

```text
Mock Atlantic server flag is enabled, starting mock server...
Starting mock Atlantic server on port 4001
Mock Atlantic server started successfully
```

**Important Notes for Orchestrator Integration:**

- The `--atlantic-service-url` value is not rewritten automatically; set it to
  `http://127.0.0.1:4001` when using the embedded mock server.
- Your actual API key, network, and other parameters are preserved and used by the mock server

## Configuration Options

The `MockServerConfig` struct allows you to customize the server behavior:

- `simulate_failures`: Enable/disable failure simulation
- `processing_delay_ms`: Time before jobs move to "InProgress" status
- `failure_rate`: Percentage of jobs that should fail (0.0 to 1.0)
- `auto_complete_jobs`: Whether jobs should automatically complete
- `completion_delay_ms`: Time before jobs move to "Done" status

## Job Lifecycle

Jobs submitted to the mock server follow this lifecycle:

1. **Received** - Initial status when job is submitted
2. **InProgress** - After `processing_delay_ms` milliseconds
3. **Done** or **Failed** - After `completion_delay_ms` milliseconds
