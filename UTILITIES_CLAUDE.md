# UTILITIES_CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) for the smaller utility folders in this repository: `cairo/`, `madaraup/`, `scripts/`, `tools/`, `test_utils/`, `build-artifacts/`, and `evm/`.

---

## Cairo (`cairo/`)

Cairo smart contracts used for genesis block initialization and testing.

### Purpose

- Smart contracts for genesis/devnet initialization
- Test contracts for JavaScript and orchestrator integration testing

### Structure

```
cairo/
├── js_tests/           # Contracts for JS integration testing
│   ├── Scarb.toml      # Cairo project manifest
│   └── src/
│       ├── lib.cairo   # Module exports
│       ├── hello.cairo # HelloStarknet example contract
│       ├── test_account.cairo
│       ├── messaging_test.cairo
│       └── appchain_test.cairo
└── orchestrator_tests/ # Mock contracts for orchestrator testing
    └── Scarb.toml
```

### Commands

```bash
# Build contracts
cd cairo/js_tests && scarb build

# Run tests
cd cairo/js_tests && snforge test
```

### Key Info

- Cairo version: 2.8.2, Scarb version: 2.8.2
- Uses OpenZeppelin contracts v0.15.1
- snforge v0.27.0 for testing
- Sierra and CASM compilation enabled

---

## Madaraup (`madaraup/`)

Cross-platform installer script for Madara binaries.

### Purpose

- Install Madara without building from source
- Update Madara versions
- Supports multiple installation modes

### Files

- `README.md`: Installation documentation
- `madaraup`: Main executable script
- `install`: Bootstrap installation script

### Commands

```bash
# Install from web
curl -L https://install.madara.build | bash

# Local installation
madaraup --path /local/madara/repo

# Install specific version
madaraup --version v0.7.0

# Install from branch/PR/tag
madaraup --branch feat/new-feature
madaraup --tag v0.7.0-beta
madaraup --pr 123
```

### Key Info

- Creates `~/.madara/bin` directory
- Updates shell profile (zsh/bash/fish/ash)
- Default version: "stable" (v0.7.0)
- Requires git and curl; Rust only for source builds

---

## Scripts (`scripts/`)

Utility scripts for development, testing, deployment, and database management.

### Key Scripts

| Script                   | Purpose                      | Usage                                                   |
| ------------------------ | ---------------------------- | ------------------------------------------------------- |
| `madara`                 | Node management              | `./madara start KEY`, `./madara reset`, `./madara lint` |
| `launcher`               | Interactive Docker launcher  | Interactive setup for full/sequencer/devnet modes       |
| `js-tests.sh`            | JavaScript integration tests | Builds madara, starts devnet, runs npm tests            |
| `e2e-tests.sh`           | E2E test runner              | Runs cargo nextest with ETH fork                        |
| `e2e-coverage.sh`        | Coverage e2e tests           | Runs with llvm-cov, generates LCOV                      |
| `artifacts.sh`           | Build contract artifacts     | Docker-based compilation                                |
| `rpc_cmp`                | RPC comparison tool          | Compare Madara vs Pathfinder responses                  |
| `create-base-db.sh`      | DB fixture creation          | Creates versioned DB snapshots                          |
| `update-version-file.sh` | Version tracking             | Updates .db-versions.yml                                |

### Common Usage

```bash
# Node management
./scripts/madara start <L1_ENDPOINT_KEY>
./scripts/madara reset
./scripts/madara lint

# Testing
./scripts/js-tests.sh
./scripts/e2e-tests.sh
./scripts/e2e-coverage.sh

# Build artifacts
./scripts/artifacts.sh

# RPC comparison
./scripts/rpc_cmp --madara http://127.0.0.1:9944 --pathfinder http://pathfinder:8080 getBlockWithTxs 1
```

### Key Info

- All scripts use `set -e` for fail-fast
- Default database path: `/tmp/madara`
- Default RPC port: 9944
- `launcher` is 870 lines with full interactive setup

---

## Tools (`tools/`)

Nix derivation files for reproducible tool installation.

### Files

| File          | Tool    | Version | Purpose                               |
| ------------- | ------- | ------- | ------------------------------------- |
| `foundry.nix` | Foundry | 1.2.3   | Solidity toolkit (anvil, cast, forge) |
| `scarb.nix`   | Scarb   | 2.8.2   | Cairo package manager                 |

### Supported Platforms

- Foundry: darwin_arm64, darwin_amd64, linux_amd64, linux_arm64, alpine, windows
- Scarb: aarch64-darwin, x86_64-darwin, linux (musl), windows

### Usage (in Nix)

```nix
let
  foundry = import ./tools/foundry.nix { inherit pkgs; };
  scarb = import ./tools/scarb.nix { inherit pkgs; };
in { ... }
```

### Key Info

- Uses pre-built binaries (no compilation)
- musl-based binaries on Linux for NixOS compatibility
- SHA256 hashes for security verification

---

## Test Utils (`test_utils/`)

Mock services and test utilities for development and testing.

### Structure

```
test_utils/
├── crates/
│   └── mock-atlantic-server/  # Mock Atlantic prover server
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── lib.rs
│           ├── server.rs     # Axum HTTP server
│           └── types.rs
└── scripts/
    └── deploy_dummy_verifier.sh  # Deploy MockGPSVerifier
```

**Atlantic API Reference:**

- Swagger UI: https://atlantic.api.herodotus.cloud/docs/
- OpenAPI JSON: https://atlantic.api.herodotus.cloud/docs/json

### Commands

```bash
# Build mock Atlantic server
cargo build --manifest-path test_utils/Cargo.toml

# Run mock Atlantic server
cargo run --bin utils-mock-atlantic-server              # Default: port 4001
cargo run --bin utils-mock-atlantic-server 8080        # Custom port
cargo run --bin utils-mock-atlantic-server 8080 0.1    # With 10% failure rate

# Deploy dummy verifier
./test_utils/scripts/deploy_dummy_verifier.sh \
  --private-key "0xac0974..." \
  --anvil-url "http://localhost:8545"
```

### Mock Atlantic API

```
POST /atlantic-query?apiKey={key}  # Submit proving job
GET  /atlantic-query/{job_id}      # Get job status
GET  /queries/{task_id}/proof.json # Download proof
GET  /health                       # Health check
```

### Key Info

- Default port: 3001
- Default API key: "mock-key"
- Job states: Received → InProgress → Done/Failed
- Configurable failure rate, delays, auto-complete

---

## Build Artifacts (`build-artifacts/`)

Pre-compiled smart contracts in JSON format.

### Structure

```
build-artifacts/
├── bootstrapper/       # MadaraFactory, EIC, Solidity contracts
├── argent/             # Argent wallet (sierra.json, casm.json)
├── braavos/            # Braavos wallet
├── js_tests/           # Compiled JS test contracts
├── orchestrator_tests/ # Orchestrator test contracts
├── starkgate_latest/   # Latest StarkGate token bridge
├── starkgate_legacy/   # Legacy StarkGate
└── cairo_lang/         # Legacy Cairo language artifacts
```

### File Formats

- `.sierra.json`: High-level Cairo (human-readable)
- `.casm.json`: Low-level assembly (executable)
- `.contract_class.json`: Contract class definitions
- `.compiled_contract_class.json`: Compiled classes

### Build Command

```bash
./scripts/artifacts.sh
```

### Key Info

- Built using Docker (Rust 1.85, Python, npm)
- Contracts compiled from source (not pre-downloaded)
- Includes account contracts from Argent and Braavos
- StarkGate contracts for token bridging

---

## EVM (`evm/`)

Docker Compose setup for local EVM-Starknet integrated testing.

### Services

| Service          | Image                     | Port  | Purpose             |
| ---------------- | ------------------------- | ----- | ------------------- |
| starknet         | Custom Madara             | 9944  | Starknet sequencer  |
| mongo            | mongo:6.0.8               | 27017 | Data persistence    |
| apibara-dna      | apibara/starknet:1.5.0    | 7171  | Data indexing       |
| indexer          | apibara/sink-mongo        | -     | Kakarot indexer     |
| kakarot-rpc      | kakarot-rpc:v0.7.1-alpha1 | 3030  | EVM RPC             |
| kakarot-deployer | kakarot:v0.9.2            | -     | Contract deployment |

### Commands

```bash
# Start all services
docker-compose up -d

# Custom configuration
MADARA_MODE=full MADARA_RPC_PORT=9945 docker-compose up -d

# With custom chain config
MADARA_CHAIN_CONFIG=/path/to/config.yaml docker-compose up -d

# Stop all
docker-compose down
```

### Environment Variables

```
MADARA_BASE_PATH: /var/lib/madara
MADARA_MODE: sequencer | full
MADARA_PRESET: mainnet | testnet | devnet
MADARA_RPC_PORT: 9944
```

### Key Info

- Kakarot provides EVM compatibility on Starknet
- Apibara streams real-time blockchain data
- Health checks ensure service readiness
- Volumes: madara_files, mongo_data

---

## Quick Reference

| Folder             | Language | Build Tool      | Purpose            |
| ------------------ | -------- | --------------- | ------------------ |
| `cairo/`           | Cairo 2  | Scarb + snforge | Smart contracts    |
| `madaraup/`        | Bash     | Native          | Installer          |
| `scripts/`         | Bash     | Native          | Dev utilities      |
| `tools/`           | Nix      | Nix             | Environment setup  |
| `test_utils/`      | Rust     | Cargo           | Mock services      |
| `build-artifacts/` | JSON     | Docker          | Compiled contracts |
| `evm/`             | YAML     | Docker Compose  | EVM integration    |

## Cross-Folder Relationships

1. **Cairo → Build-Artifacts**: Cairo contracts compiled to JSON artifacts
2. **Scripts → Build-Artifacts**: `scripts/artifacts.sh` rebuilds artifacts
3. **Scripts → Cairo**: `scripts/js-tests.sh` uses Cairo test contracts
4. **Test-Utils → Scripts**: Mock Atlantic used by orchestrator tests
5. **Tools → Development**: Nix configs ensure Foundry/Scarb availability
6. **EVM → Cairo**: Kakarot wraps Starknet contracts for EVM access
