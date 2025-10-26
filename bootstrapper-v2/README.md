# Madara Bootstrapper v2

A CLI application for bootstrapping complete Madara networks with Ethereum as the base layer.
Deploys both base layer and Madara contracts using factory patterns for efficient, atomic deployments.

## Features

- **Two-Phase Deployment**: L1 setup → L2 setup with automatic bridge configuration
- **Factory Pattern**: Atomic contract deployments using factory contracts
- **Bridge Infrastructure**: Complete cross-layer communication setup
- **Address Management**: Automatic generation and management of contract addresses

## Architecture

**Deployment Flow:**

1. **Base Layer Setup**: Deploy Ethereum implementation contracts → Deploy factory → Deploy core contracts
2. **Madara Setup**: Deploy Cairo contracts → Deploy Madara factory → Deploy L2 bridges → Connect L1↔L2

**Contracts:**

- **Base Layer**: CoreContract, Manager, Registry, MultiBridge, EthBridge, EthBridgeEIC + Factory
- **Madara Setup**: TokenBridge, ERC20, EIC, UniversalDeployer, MadaraFactory + L2 Bridges

## Project Structure

```text
bootstrapper-v2/
├── src/
│   ├── cli/                    # CLI commands
│   ├── setup/
│   │   ├── base_layer/ethereum/ # L1 setup
│   │   └── madara/             # L2 setup
│   └── config.rs               # Configuration
├── contracts/
│   ├── ethereum/               # Solidity contracts
│   └── madara/                 # Cairo contracts
├── configs/                    # Configuration files
└── output/                     # Generated addresses
```

### Build

```bash
cargo build --release
```

### Compatibility

⚠️ **Important**: This bootstrapper is **not backwards compatible** and requires:

- **Madara node** running **StarkNet protocol version 0.14.0** (fully compatible)

## Usage

### Environment Setup

Set required environment variables:

```bash
export BASE_LAYER_PRIVATE_KEY="your_base_layer_private_key"
export MADARA_PRIVATE_KEY="your_madara_private_key"
```

### Setup Workflow

#### Step 1: Setup Base Layer

```bash
RUST_LOG=debug && cargo run --bin bootstrapper-v2 -- \
  setup-base --config-path configs/config.json \
  --addresses-output-path output/addresses.json
```

#### Step 2: Setup Madara

```bash
RUST_LOG=debug cargo run -- \
  setup-madara --config-path configs/config.json \
  --base-addresses-path output/addresses.json \
  --output-path output/madara_addresses.json
```

**⚠️ Important**: `setup-base` must be run before `setup-madara`.

### Configuration

Update `configs/config.json` with your RPC URLs:

```json
{
  "base_layer": {
    "layer": "ETHEREUM",
    "rpc_url": "http://localhost:8545",
    "implementation_addresses": {},
    "core_contract_init_data": {
      /* ... */
    }
  },
  "madara": {
    "rpc_url": "http://localhost:9944"
  }
}
```

**Note**: `madara.rpc_url` must point to a running Madara node.

## Troubleshooting

**Common Issues:**

- **RPC Connection Failed**: Ensure Madara/Ethereum nodes are running
- **Missing L1 Addresses**: Ensure `setup-base` completed successfully
