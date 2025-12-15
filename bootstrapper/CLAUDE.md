# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in the bootstrapper.

## Project Overview

The Madara Bootstrapper is a Rust-based tool for deploying and initializing bridge contracts
between a Madara/Starknet Appchain (L2) and Ethereum/EVM-compatible chains (L1). It automates
the complex setup process for cross-chain interoperability.

**Key capabilities:**

- Deploy Token Bridge and ETH Bridge contracts between L2 (Madara) and L1 (Ethereum/EVM)
- Declare wallet contracts from OpenZeppelin, Argent, and Braavos
- Set up essential infrastructure contracts (UDC, Core Contract)
- Manage configuration across two different blockchain networks
- Support both standalone setup for individual components and full end-to-end bootstrapping

**Note:** There is also `bootstrapper-v2/` which uses a factory pattern for atomic deployments. See the bootstrapper-v2 section below.

## Common Commands

### Building

```bash
cargo build --release
```

### Running

```bash
# Show help
RUST_LOG=info cargo run -- --help

# Dev mode (unsafe proxy, minimal setup)
RUST_LOG=info cargo run -- --dev

# With specific config file
RUST_LOG=info cargo run -- --mode setup-l1 --config src/configs/devnet.json

# Save output to file
RUST_LOG=info cargo run -- --mode setup-l1 --output-file output.json
```

### Setup Sequence

```bash
# 1. Setup L1 (outputs core contract addresses)
RUST_LOG=debug cargo run --release -- --mode setup-l1 --config src/configs/devnet.json

# 2. Update config with returned addresses (CORE_CONTRACT_ADDRESS, CORE_CONTRACT_IMPLEMENTATION_ADDRESS)

# 3. Setup L2 (deploys all L2 components)
RUST_LOG=debug cargo run --release -- --mode setup-l2 --config src/configs/devnet.json
```

### Docker

```bash
cp .env.example .env  # Configure environment
docker compose build
docker compose up      # Remote networks
# OR
docker compose -f docker-compose-local.yml up  # Local networks
```

### Testing

```bash
# Tests require Madara + Anvil running
# First uncomment #[ignore] tags in src/tests/mod.rs
RUST_LOG=debug cargo test -- --nocapture

# Individual tests (one at a time):
cargo test deploy_bridge -- --nocapture
cargo test deposit_and_withdraw_eth_bridge -- --nocapture
cargo test deposit_and_claim_erc20 -- --nocapture
```

## Architecture Overview

### Bootstrap Modes

| Mode               | Purpose                                   |
| ------------------ | ----------------------------------------- |
| `Core` / `SetupL1` | Deploy core contract on L1                |
| `SetupL2`          | Complete L2 setup with all components     |
| `EthBridge`        | Deploy ETH bridge only                    |
| `Erc20Bridge`      | Deploy ERC20 token bridge only            |
| `Udc`              | Deploy Universal Deployer Contract        |
| `Argent`           | Declare Argent wallet contracts           |
| `Braavos`          | Declare Braavos wallet contracts          |
| `UpgradeEthBridge` | Upgrade ETH bridge to newer Cairo version |

### Module Structure

**`contract_clients/`** - Contract interaction abstractions

- `config.rs`: Client initialization (EthereumClient, L2 providers)
- `core_contract.rs`: Core contract trait definition
- `starknet_core_contract.rs`: Production core contract (safe proxies)
- `starknet_dev_core_contract.rs`: Development core contract (unsafe proxies)
- `eth_bridge.rs`: ETH bridge contract client
- `token_bridge.rs`: ERC20 token bridge client
- `utils.rs`: Account utilities, declaration functions

**`setup_scripts/`** - Deployment orchestration

- `account_setup.rs`: User account initialization on L2
- `core_contract.rs`: Core contract deployment and initialization
- `eth_bridge.rs`: ETH bridge infrastructure deployment
- `erc20_bridge.rs`: Token bridge deployment
- `udc.rs`: Universal Deployer Contract deployment
- `argent.rs`: Argent wallet declaration
- `braavos.rs`: Braavos wallet declaration
- `upgrade_*.rs`: Upgrade scripts for various components

**`helpers/`** - Account interaction utilities

- `account_actions.rs`: Contract invocation, declarations, transfers

**`transport.rs`** - Admin RPC bypass transport

- Enables fee-less transactions during bootstrapping
- Supports Cairo 0 declarations via admin RPC

**`utils/`** - Shared utilities

- `constants.rs`: Contract artifact paths
- `mod.rs`: Common helpers (invoke, wait, save to JSON)

## Configuration

### Required Environment Variables

**L1 (Ethereum):**

```text
eth_rpc: Ethereum RPC endpoint (default: http://127.0.0.1:8545)
eth_priv_key: Private key for L1 deployer
eth_chain_id: Chain ID (e.g., 31337 for local anvil)
l1_deployer_address: Deployer account address
```

**L2 (Starknet/Madara):**

```text
rollup_seq_url: L2 sequencer URL (default: http://127.0.0.1:19944/rpc/v0.8.1/)
rollup_declare_v0_seq_url: Admin RPC for Cairo 0 declarations (default: http://127.0.0.1:19943)
rollup_priv_key: L2 deployer private key
app_chain_id: Starknet chain ID (default: MADARA_DEVNET)
```

**Governance:**

```text
l1_multisig_address: L1 multisig for governance
l2_multisig_address: L2 multisig for governance
verifier_address: Cairo proof verifier address
operator_address: Block proposer/operator address
```

### Output

- Addresses saved to `data/addresses.json`
- Includes all deployed contract addresses

## Important Implementation Notes

### When Working with Contracts

- Use `declare_contract()` for both Cairo 0 (legacy) and Cairo 1 (Sierra) contracts
- Admin RPC bypass enables fee-less transactions during bootstrap
- Safe proxies for production, unsafe proxies for development

### When Working with Bridges

- ETH Bridge uses proxy pattern with initialization
- ERC20 Bridge includes Manager, Registry, and Bridge components
- Test DAI token deployed for testing ERC20 functionality

### When Adding New Wallet Support

1. Add declaration function in `setup_scripts/`
2. Create constant paths in `utils/constants.rs`
3. Register in appropriate bootstrap mode

---

## Bootstrapper V2

Located in `../bootstrapper-v2/`, this is a newer implementation using factory patterns for
atomic deployments.

## Key Differences from V1

- **Factory Pattern**: Atomic deployments via Factory contracts on both L1 and L2
- **Two-Phase Setup**: Separate `setup-base` and `setup-madara` commands
- **Automatic Bridge Configuration**: Post-setup linking between layers
- **Version Requirement**: Requires Madara with StarkNet protocol version 0.14.0

## V2 Commands

```bash
# Setup Base Layer (L1)
RUST_LOG=debug cargo run --bin bootstrapper-v2 -- \
  setup-base --config-path configs/config.json \
  --addresses-output-path output/addresses.json

# Setup Madara (L2) - requires base layer addresses
RUST_LOG=debug cargo run -- \
  setup-madara --config-path configs/config.json \
  --base-addresses-path output/addresses.json \
  --output-path output/madara_addresses.json
```

### V2 Environment Variables

```text
BASE_LAYER_PRIVATE_KEY=0xabcd    # Private key for L1 deployments
MADARA_PRIVATE_KEY=0xabcd        # Private key for L2 deployments
RUST_LOG=info                    # Logging level
```

## V2 Architecture

**Base Layer (L1) Deployment:**

1. Deploy implementation contracts
2. Deploy Factory contract
3. Call Factory.setup() → deploys CoreContract, Manager, Registry, MultiBridge, EthBridge
4. Post-Madara: Update L2 bridge addresses

**Madara (L2) Deployment:**

1. Bootstrap account declaration
2. Deploy user account via OpenZeppelin AccountFactory
3. Declare all Cairo contracts
4. Deploy UniversalDeployer (UDC)
5. Deploy MadaraFactory
6. Call MadaraFactory.deploy_bridges() → deploys L2 ETH Token, ETH Bridge, Token Bridge

## V2 Output Files

- `output/addresses.json`: Base layer addresses (CoreContract, bridges, manager, registry)
- `output/madara_addresses.json`: Madara addresses (factory, UDC, bridges, tokens)
