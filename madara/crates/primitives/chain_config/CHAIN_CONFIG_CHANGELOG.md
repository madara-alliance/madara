# Chain Config Changelog

This document tracks all versions of the Madara chain configuration format and documents breaking changes between versions.

## Version 1 (Current)

**Release Date:** TBD

This is the initial versioned release of the chain config format. All chain configs must now include a `config_version` field.

### Major Changes

#### L2 Gas Price Configuration Refactor

The L2 gas price configuration has been refactored from multiple separate fields into a single enum-based configuration.
This provides a clearer and more explicit way to specify gas pricing strategy.

**Old Format (Deprecated):**

```yaml
l2_gas_target: 2000000000
min_l2_gas_price: 100000
l2_gas_price_max_change_denominator: 48
```

**New Format:**

Option 1 - EIP-1559 Dynamic Pricing:

```yaml
l2_gas_price:
  type: eip1559
  target: 2000000000
  min_price: 100000
  max_change_denominator: 48
```

Option 2 - Fixed Gas Price:

```yaml
l2_gas_price:
  type: fixed
  price: 2500
```

The new format makes it explicit which pricing strategy is being used, preventing configuration errors.

### Required Fields

- `config_version` (u32): The version of the chain config format. Must be set to `1` for this version.
- `chain_name` (String): Human readable chain name
- `chain_id` (String): Chain ID (e.g., "MAINNET", "SEPOLIA")
- `feeder_gateway_url` (URL): Feeder gateway endpoint URL
- `gateway_url` (URL): Gateway endpoint URL
- `native_fee_token_address` (Address): STRK token address
- `parent_fee_token_address` (Address): ETH token address
- `eth_core_contract_address` (String): Starknet core contract address on L1
- `eth_gps_statement_verifier` (String): GPS statement verifier address on L1
- `sequencer_address` (Address): Sequencer address for block production
- `mempool_max_transactions` (usize): Maximum number of transactions in mempool
- `l2_gas_price`: L2 gas pricing configuration
  - For EIP-1559: `type: eip1559`, `target`, `min_price`, `max_change_denominator`
  - For fixed: `type: fixed`, `price`

### Optional Fields

- `l1_da_mode` (default: Blob): L1 data availability mode
- `settlement_chain_kind` (default: Ethereum): Settlement chain kind
- `versioned_constants` (default: built-in): Protocol versioned constants
- `versioned_constants_path` (map): Custom versioned constants file paths
- `latest_protocol_version` (default: latest): Starknet protocol version
- `block_time` (default: 30s): Block production time
- `no_empty_blocks` (default: false): Skip empty block production
- `bouncer_config` (default: default): Block size limits configuration
- `mempool_mode` (default: Timestamp): Mempool ordering mode
- `mempool_min_tip_bump` (default: 0.1): Minimum tip increase ratio
- `mempool_max_declare_transactions` (optional): Max declare transactions limit
- `mempool_ttl` (optional): Transaction time-to-live in mempool
- `block_production_concurrency` (default: auto): Parallel execution config
- `l1_messages_replay_max_duration` (default: 3 days): L1 message replay duration

### Breaking Changes

- **BREAKING:** `config_version` field is now required. All chain configs must specify this field.
- Chain configs without a `config_version` field will fail to load with an error.
- **BREAKING:** L2 gas price configuration has been refactored. The following fields have been removed:
  - `l2_gas_target` → Use `l2_gas_price.target` (in EIP-1559 mode)
  - `min_l2_gas_price` → Use `l2_gas_price.min_price` (in EIP-1559 mode)
  - `l2_gas_price_max_change_denominator` → Use `l2_gas_price.max_change_denominator` (in EIP-1559 mode)

### Migration Guide

To migrate from a legacy (unversioned) chain config to version 1:

1. Add the following line to the top of your chain config YAML file (after any comments):

   ```yaml
   config_version: 1
   ```

2. Update L2 gas price configuration:

   **If you were using dynamic pricing (default):**

   Replace:

   ```yaml
   l2_gas_target: 2000000000
   min_l2_gas_price: 100000
   l2_gas_price_max_change_denominator: 48
   ```

   With:

   ```yaml
   l2_gas_price:
     type: eip1559
     target: 2000000000
     min_price: 100000
     max_change_denominator: 48
   ```

   **(NEW FEAT) If you want to use fixed pricing:**

   Use:

   ```yaml
   l2_gas_price:
     type: fixed
     price: 2500
   ```

3. Verify your config loads correctly:

   ```bash
   madara --chain-config-path /path/to/your/config.yaml --help
   ```

### Example

```yaml
config_version: 1
chain_name: "My Custom Chain"
chain_id: "MY_CHAIN"
feeder_gateway_url: "https://example.com/feeder_gateway/"
gateway_url: "https://example.com/gateway/"
# ... other required fields ...

# Using EIP-1559 dynamic pricing
l2_gas_price:
  type: eip1559
  target: 2000000000
  min_price: 100000
  max_change_denominator: 48
# Or using fixed pricing
# l2_gas_price:
#   type: fixed
#   price: 2500
```

---

## Version History

| Version | Release Date | Status  |
| ------- | ------------ | ------- |
| 1       | TBD          | Current |

## Future Versions

Future breaking changes to the chain config format will increment the version number. This may include:

- Adding new required fields
- Removing fields
- Changing field types or validation rules
- Changing field semantics

All future versions will be documented here with migration guides.
