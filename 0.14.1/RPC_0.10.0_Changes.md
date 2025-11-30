# RPC 0.10.0 Specification Changes

## Overview

Starknet v0.14.1 introduces JSON-RPC v0.10.0 with several breaking changes and new features. This document details all changes required for Madara's RPC implementation.

## Current RPC Version

Madara currently supports:

- v0.7.1
- v0.8.1
- v0.9.0

**New Version Required**: v0.10.0

## Changes Summary

1. **State Diff Extended**: Added `migrated_compiled_classes` field
2. **Preconfirmed Block Optimization**: Removed `old_root` from `PRE_CONFIRMED_STATE_UPDATE`
3. **Event Context Enriched**: Added `transaction_index` and `event_index` to events
4. **Error Updates**: Added `CONTRACT_NOT_FOUND` error for fee estimation methods
5. **Type Changes**: Changed `storage_keys` parameter type from `FELT` to `STORAGE_KEY`
6. **Subscription Updates**: `subscribeReorg` now fires on `NewTransaction` and `NewTransactionReceipts`

## Detailed Changes

### 1. State Diff Extended: `migrated_compiled_classes`

#### Current Implementation

**Location**: `madara/crates/primitives/state_update/src/lib.rs`

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StateDiff {
    pub storage_diffs: Vec<ContractStorageDiffItem>,
    pub deployed_contracts: Vec<DeployedContractItem>,
    pub old_declared_contracts: Vec<Felt>,
    pub declared_classes: Vec<DeclaredClassItem>,
    pub nonces: Vec<NonceUpdate>,
    pub replaced_classes: Vec<DeployedContractItem>,
}
```

#### Required Change

Add `migrated_compiled_classes` field:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StateDiff {
    // ... existing fields
    pub migrated_compiled_classes: Vec<MigratedClassItem>, // NEW
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MigratedClassItem {
    pub class_hash: Felt,
    pub compiled_class_hash: Felt, // New BLAKE hash (post-SNIP-34)
}
```

#### Implementation Locations

1. **State Diff Construction**: `madara/crates/primitives/state_update/src/lib.rs`
   - Add field to `StateDiff` struct
   - Populate during state diff creation

2. **Block Production**: `madara/crates/client/block_production/src/lib.rs`
   - Include migrated classes when creating state diff
   - Track classes that were migrated (Poseidon → BLAKE)

3. **RPC Response**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_state_update.rs`
   - Include `migrated_compiled_classes` in response

4. **Gateway Sync**: `madara/crates/client/sync/src/gateway/blocks.rs`
   - Parse `migrated_compiled_classes` from gateway responses

### 2. Preconfirmed Block Optimization: Remove `old_root`

#### Current Implementation

**Location**: `madara/crates/client/rpc/src/versions/user/v0_9_0/methods/read/get_state_update.rs`

```rust
pub struct PreConfirmedStateUpdate {
    pub old_root: Felt,  // REMOVE THIS
    pub state_diff: StateDiff,
}
```

#### Required Change

Remove `old_root` field from `PreConfirmedStateUpdate`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreConfirmedStateUpdate {
    // old_root: Felt, // REMOVED in v0.10.0
    pub state_diff: StateDiff,
}
```

#### Implementation Locations

1. **RPC Types**: `madara/crates/primitives/rpc/src/v0_10_0/starknet_api_openrpc.json`
   - Update OpenRPC schema

2. **RPC Implementation**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_state_update.rs`
   - Remove `old_root` from response construction

3. **Type Definitions**: `madara/crates/primitives/rpc/src/v0_10_0/mod.rs`
   - Update `PreConfirmedStateUpdate` struct

**Note**: `StateUpdate` (for confirmed blocks) still includes `old_root`.

### 3. Event Context Enriched: `transaction_index` and `event_index`

#### Current Implementation

**Location**: `madara/crates/client/rpc/src/versions/user/v0_9_0/methods/read/get_events.rs`

Events currently include:

- `event`: Event data
- `block_hash`: Block hash
- `block_number`: Block number

#### Required Change

Add `transaction_index` and `event_index` to each event:

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EmittedEvent {
    pub event: Event,
    pub block_hash: Felt,
    pub block_number: u64,
    pub transaction_index: u64,  // NEW
    pub event_index: u64,         // NEW
}
```

#### Implementation Locations

1. **Event Storage**: `madara/crates/client/db/src/events.rs`
   - Store `transaction_index` and `event_index` with events

2. **Event Retrieval**: `madara/crates/client/db/src/storage.rs`
   - Return `transaction_index` and `event_index` in `EventWithInfo`

3. **RPC Response**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_events.rs`
   - Include indices in response

4. **Event Conversion**: `madara/crates/primitives/block/src/event.rs`
   - Update `EventWithInfo` to include indices

**Current Structure**: `madara/crates/primitives/block/src/lib.rs`

```rust
pub struct EventWithInfo {
    pub event: Event,
    pub block_number: u64,
    pub event_index_in_block: usize, // Already exists!
    pub transaction_hash: Felt,
    // Need to add: transaction_index
}
```

### 4. Error Updates: `CONTRACT_NOT_FOUND`

#### Current Implementation

**Location**: `madara/crates/client/rpc/src/versions/user/v0_9_0/methods/read/estimate_fee.rs`

Current errors:

- `BlockNotFound`
- `TxnExecutionError`
- `UnsupportedTxnVersion`

#### Required Change

Add `CONTRACT_NOT_FOUND` error for:

- `starknet_estimateFee`
- `starknet_estimateMessageFee`

#### Implementation Locations

1. **Error Definitions**: `madara/crates/client/rpc/src/errors.rs`

```rust
#[derive(Debug, thiserror::Error)]
pub enum StarknetRpcApiError {
    // ... existing errors
    #[error("Contract not found")]
    ContractNotFound, // NEW
}
```

2. **Fee Estimation**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/estimate_fee.rs`

```rust
pub async fn estimate_fee(...) -> StarknetRpcResult<Vec<FeeEstimate>> {
    // Check if contract exists before execution
    for tx in &transactions {
        if let Some(contract_address) = tx.contract_address() {
            if !view.is_contract_deployed_at(contract_address)? {
                return Err(StarknetRpcApiError::ContractNotFound);
            }
        }
    }
    // ... rest of function
}
```

3. **Message Fee Estimation**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/estimate_message_fee.rs`

```rust
pub async fn estimate_message_fee(...) -> StarknetRpcResult<MessageFeeEstimate> {
    // Check if L1 handler contract exists
    if !view.is_contract_deployed_at(&message.contract_address)? {
        return Err(StarknetRpcApiError::ContractNotFound);
    }
    // ... rest of function
}
```

### 5. Type Changes: `storage_keys` Parameter

#### Current Implementation

**Location**: `madara/crates/client/rpc/src/versions/user/v0_8_1/methods/read/get_storage_proof.rs`

Current parameter type: `FELT`

#### Required Change

Change `storage_keys` parameter type from `FELT` to `STORAGE_KEY`.

**Note**: `STORAGE_KEY` is likely a type alias or wrapper around `FELT` for semantic clarity. Check RPC spec for exact definition.

#### Implementation Locations

1. **RPC Types**: `madara/crates/primitives/rpc/src/v0_10_0/starknet_api_openrpc.json`
   - Update parameter type in schema

2. **RPC Implementation**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/read/get_storage_proof.rs`
   - Update function signature

3. **Type Definitions**: `madara/crates/primitives/rpc/src/v0_10_0/mod.rs`
   - Define `STORAGE_KEY` type (if not already defined)

### 6. Subscription Updates: `subscribeReorg`

#### Current Implementation

**Location**: `madara/crates/client/rpc/src/versions/user/v0_8_1/methods/ws/`

Current behavior: `subscribeReorg` fires only on chain reorganizations.

#### Required Change

`subscribeReorg` should also fire on:

- `NewTransaction`
- `NewTransactionReceipts`

#### Implementation Locations

1. **WebSocket Subscriptions**: `madara/crates/client/rpc/src/versions/user/v0_10_0/methods/ws/subscribe_reorg.rs`

```rust
pub async fn subscribe_reorg(
    starknet: &Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await?;

    // Subscribe to reorg events
    let mut reorg_rx = starknet.backend.subscribe_reorg();

    // ALSO subscribe to new transactions and receipts
    let mut tx_rx = starknet.backend.subscribe_new_transactions();
    let mut receipt_rx = starknet.backend.subscribe_new_receipts();

    tokio::select! {
        reorg = reorg_rx.recv() => {
            send_reorg_event(reorg, &sink).await?;
        }
        tx = tx_rx.recv() => {
            send_reorg_event_for_transaction(tx, &sink).await?;
        }
        receipt = receipt_rx.recv() => {
            send_reorg_event_for_receipt(receipt, &sink).await?;
        }
    }
}
```

2. **Backend Subscriptions**: `madara/crates/client/db/src/subscription.rs`
   - Add `subscribe_new_transactions()` and `subscribe_new_receipts()` methods

3. **Block Production**: `madara/crates/client/block_production/src/lib.rs`
   - Notify subscribers when new transactions/receipts are added

## RPC Version Structure

### Create v0.10.0 Module

**New Directory**: `madara/crates/client/rpc/src/versions/user/v0_10_0/`

Structure:

```
v0_10_0/
├── mod.rs
├── methods/
│   ├── mod.rs
│   ├── read/
│   │   ├── mod.rs
│   │   ├── get_events.rs          # Updated with indices
│   │   ├── get_state_update.rs     # Updated with migrated_classes, no old_root for preconfirmed
│   │   ├── estimate_fee.rs         # Updated with CONTRACT_NOT_FOUND
│   │   ├── estimate_message_fee.rs # Updated with CONTRACT_NOT_FOUND
│   │   └── get_storage_proof.rs    # Updated storage_keys type
│   ├── ws/
│   │   ├── mod.rs
│   │   └── subscribe_reorg.rs      # Updated to fire on tx/receipts
│   └── ...
```

### Update RPC Module Registration

**Location**: `madara/node/src/service/rpc/mod.rs`

Add v0.10.0 to RPC module:

```rust
let v0_10_0_module = versions::user::v0_10_0::create_rpc_module(starknet.clone());
module.merge(v0_10_0_module)?;
```

## OpenRPC Schema Updates

**Location**: `madara/crates/primitives/rpc/src/v0_10_0/starknet_api_openrpc.json`

Update schema file with:

1. New `migrated_compiled_classes` field in state diff
2. Removed `old_root` from `PreConfirmedStateUpdate`
3. Added `transaction_index` and `event_index` to events
4. Added `CONTRACT_NOT_FOUND` error code
5. Changed `storage_keys` type to `STORAGE_KEY`
6. Updated `subscribeReorg` description

## Testing Requirements

### Unit Tests

1. **State Diff**: Test `migrated_compiled_classes` serialization/deserialization
2. **Preconfirmed State Update**: Test `old_root` is not included
3. **Events**: Test `transaction_index` and `event_index` are included
4. **Errors**: Test `CONTRACT_NOT_FOUND` is returned correctly
5. **Subscriptions**: Test `subscribeReorg` fires on transactions/receipts

### Integration Tests

1. **RPC Endpoints**: Test all updated endpoints return correct format
2. **Backward Compatibility**: Test v0.9.0 endpoints still work
3. **Cross-Version**: Test clients can use both v0.9.0 and v0.10.0

## Backward Compatibility

### Version Support

- **v0.9.0**: Keep existing implementation (no changes)
- **v0.10.0**: New implementation with all changes
- **Default**: Can be configured (default to v0.10.0 after activation)

### Migration Path

1. Implement v0.10.0 alongside v0.9.0
2. Clients can gradually migrate to v0.10.0
3. Maintain v0.9.0 for backward compatibility

## References

- [RPC 0.10.0 Spec Repository](https://github.com/starkware-libs/starknet-specs/tree/release/v0.10.0-rc.1)
- [Pathfinder PR #3068](https://github.com/eqlabs/pathfinder/pull/3068) - State-update changes
- [Pathfinder PR #3091](https://github.com/eqlabs/pathfinder/pull/3091) - Migrated classes support
