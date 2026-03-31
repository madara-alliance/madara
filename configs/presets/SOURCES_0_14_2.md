# Preset Sources For Starknet 0.14.2

These are the upstream references for the `0.14.2` preset updates in this branch.

## Protocol Version

- `latest_protocol_version: "0.14.2"`
  - Local wiring: `madara/crates/primitives/chain_config/src/chain_config.rs`
  - Upstream constants: <https://github.com/starkware-libs/sequencer/blob/7c1654b0c2753f942023265edb52a85a5f0f74ee/crates/blockifier/resources/blockifier_versioned_constants_0_14_2.json>

## Bouncer Capacity And Builtin Weights

The preset values for:

- `bouncer_config.block_max_capacity.proving_gas`
- `bouncer_config.block_max_capacity.receipt_l2_gas`
- `bouncer_config.builtin_weights.gas_costs.*`

come from the upstream `blockifier` defaults introduced for `0.14.2`:

- <https://github.com/starkware-libs/sequencer/blob/7c1654b0c2753f942023265edb52a85a5f0f74ee/crates/blockifier/src/bouncer.rs>

The `receipt_l2_gas` limit is expected to stay aligned with the orchestrator block-size limit:

- <https://github.com/starkware-libs/sequencer/blob/7c1654b0c2753f942023265edb52a85a5f0f74ee/crates/apollo_consensus_orchestrator/resources/orchestrator_versioned_constants_0_14_2.json>

## Ethereum Contract Addresses

The updated `eth_core_contract_address` / `eth_gps_statement_verifier` values are mirrored in Madara’s chain config constants:

- `madara/crates/primitives/chain_config/src/chain_config.rs`

The Starknet verifier background docs are here:

- <https://docs.starknet.io/architecture-and-concepts/solidity-verifier/>
