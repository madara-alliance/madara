# Mock Atlantic Server

A mock HTTP server that implements the Atlantic API endpoints for testing and development purposes.

## Overview

The Mock Atlantic Server provides the same HTTP interface as the real Atlantic service, allowing you to test your
applications without depending on the external Atlantic service. It supports all the major endpoints and can simulate
various scenarios including failures and processing delays.

## Features

- 🚀 **Full API Compatibility**: Implements all Atlantic API endpoints
- 🎭 **Configurable Behavior**: Simulate failures, processing delays, and different response patterns
- 🔄 **Realistic Job Lifecycle**: Jobs progress through realistic status transitions
- 📊 **Configurable Failure Rates**: Test error handling with controlled failure simulation
- 🏥 **Health Checks**: Built-in health check endpoint for monitoring
- 🧪 **Testing Support**: Easy integration for unit and integration tests

// Usage examples in comments:
//
// Run with default settings (port 4001, no failures):
// cargo run --bin mock-atlantic-server
//
// Run on port 8080:
// cargo run --bin mock-atlantic-server 8080
//
// Run on port 8080 with 10% failure rate:
// cargo run --bin mock-atlantic-server 8080 0.1

## How to Spin the Server

- Run with default settings (port 4001, no failures):

  ```bash
  cargo run --bin utils-mock-atlantic-server
  ```

- Run on port 8080:

  ```bash
  cargo run --bin utils-mock-atlantic-server 8080
  ```

- Run on port 8080 with 10% failure rate:

  ```bash
  cargo run --bin utils-mock-atlantic-server 8080 0.1
  ```

## API Endpoints

### Job Management

- `POST /atlantic-query?apiKey={key}` - Submit a new proving job
- `GET /atlantic-query/{job_id}` - Get job status and details

### Proof Retrieval

- `GET /queries/{task_id}/proof.json` - Download proof data for completed jobs

### Health & Monitoring

- `GET /health` - Health check endpoint

### Documentation

For complete API documentation, see the [swagger.json](./swagger.json) file which contains the full OpenAPI 3.0.3
specification from the Herodotus Atlantic API.

## Usage

### Running as a Standalone Server

```bash
# Run with default settings (port 3001, no failures)
cargo run --bin mock-atlantic-server

# Run on custom port
cargo run --bin mock-atlantic-server 8080

# Run with failure simulation (10% failure rate)
cargo run --bin mock-atlantic-server 8080 0.1
```

### Using with Atlantic Client

To use the mock server with the real Atlantic client, simply point the `atlantic_service_url` to your mock server when
the mock factor is enabled.

### Using with Orchestrator

The mock Atlantic server is automatically integrated with the orchestrator. When you set `atlantic_mock_fact_hash=true`,
the orchestrator will:

1. Automatically start the mock Atlantic server on port 3001
2. Configure the Atlantic client to use the local mock server instead of the real Atlantic service
3. Disable fact checking for faster testing

Example usage:

```bash
# Run orchestrator with mock Atlantic server (SIMPLIFIED!)
# Only 2 Atlantic parameters needed - everything else gets sensible defaults!
cargo run --bin orchestrator -- run \
  --atlantic \
  --atlantic-mock-fact-hash "true" \
  # ... other orchestrator arguments (non-Atlantic)

# Optional: Override defaults if needed
cargo run --bin orchestrator -- run \
  --atlantic \
  --atlantic-mock-fact-hash "true" \
  --atlantic-api-key "custom-mock-key" \
  --atlantic-network "MAINNET" \
  # ... other orchestrator arguments
```

When `atlantic_mock_fact_hash=true`, the orchestrator will log:

```text
Starting Mock Atlantic Server for testing...
Mock Atlantic Server started on port 3001
Configured Atlantic client to use mock server at http://127.0.0.1:3001
Using hardcoded verifier contract address for mock mode: 0x0000000000000000000000000000000000000000
```

**Important Notes for Orchestrator Integration:**

- The `--atlantic-service-url` you provide on the command line is automatically overridden to
  `http://127.0.0.1:3001`
- The `--atlantic-verifier-contract-address` is automatically overridden to
  `0x0000000000000000000000000000000000000000`
- Your actual API key, network, and other parameters are preserved and used by the mock server
- This ensures seamless testing without requiring manual URL changes

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
