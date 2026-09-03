//! Starknet RPC module. Implements the JSON-RPC server for Madara, providing a standardized
//! interface for interacting with the Starknet node. This module implements the official Starknet
//! JSON-RPC specification along with some Madara-specific extensions.
//!
//! Madara fully supports the Starknet JSON-RPC specification versions `v0.7.1`, `v0.8.1`, `v0.9.0`,
//! `v0.10.0`, and `v0.10.2`, with methods accessible through port **9944** by default
//! (configurable via `--rpc-port`). The RPC
//! server supports both HTTP and WebSocket connections on the same port.
//!
//! ## Version Management
//!
//! RPC methods are versioned to ensure backward compatibility. To access methods from a specific
//! version, append `/rpc/v.../` to your RPC url, where `v...` is your version code. For example:
//!
//! - Default (latest, currently v0.10.2): `http://localhost:9944/`
//! - Version 0.8.1: `http://localhost:9944/rpc/v0_8_1/`
//! - Version 0.9.0: `http://localhost:9944/rpc/v0_9_0/`
//! - Version 0.10.0: `http://localhost:9944/rpc/v0_10_0/`
//! - Version 0.10.2: `http://localhost:9944/rpc/v0_10_2/`
//!
//! ## Available Endpoints
//!
//! Below is a comprehensive list of all RPC endpoints implemented in Madara, organized by category.
//! Each method follows the Starknet JSON-RPC specification unless otherwise noted. You can find
//! more information on the official [Starknet RPC Specs] repo.
//!
//! ### Read Methods
//!
//! These methods provide read-only access to blockchain data without modifying state.
//!
//! #### `starknet_specVersion`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::spec_version`]
//! [`versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server::spec_version`]
//!
//! Returns the version of the Starknet JSON-RPC specification being used.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_specVersion",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_blockNumber`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::block_number`]
//!
//! Returns the most recent accepted block number.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_blockNumber",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_blockHashAndNumber`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::block_hash_and_number`]
//!
//! Returns the most recent accepted block hash and number.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_blockHashAndNumber",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getBlockWithTxHashes`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_block_with_tx_hashes`]
//!
//! Returns block information with transaction hashes.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getBlockWithTxHashes",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getBlockWithTxs`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_block_with_txs`]
//!
//! Returns block information with full transaction objects.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getBlockWithTxs",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getBlockWithReceipts`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_block_with_receipts`]
//!
//! Returns block information with transaction receipts.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getBlockWithReceipts",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getStateUpdate`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_state_update`]
//!
//! Returns the state changes in a given block.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getStateUpdate",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getStorageAt`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_storage_at`]
//!
//! Returns the value of a storage variable at a given block, address and key.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getStorageAt",
//!   "params": {
//!     "contract_address": "0x123...",
//!     "key": "0x456...",
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getTransactionByHash`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_transaction_by_hash`]
//!
//! Returns transaction information by transaction hash.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getTransactionByHash",
//!   "params": {
//!     "transaction_hash": "0x789..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getTransactionByBlockIdAndIndex`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_transaction_by_block_id_and_index`]
//!
//! Returns transaction information by block ID and index within the block.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getTransactionByBlockIdAndIndex",
//!   "params": {
//!     "block_id": "latest",
//!     "index": 0
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getTransactionReceipt`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_transaction_receipt`]
//!
//! Returns the receipt of a transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getTransactionReceipt",
//!   "params": {
//!     "transaction_hash": "0x789..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getTransactionStatus`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_transaction_status`]
//!
//! Returns the execution and finality status of a transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getTransactionStatus",
//!   "params": {
//!     "transaction_hash": "0x789..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getClass`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_class`]
//!
//! Returns the contract class definition at a given class hash.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getClass",
//!   "params": {
//!     "block_id": "latest",
//!     "class_hash": "0xabc..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getClassHashAt`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_class_hash_at`]
//!
//! Returns the class hash deployed at a given contract address.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getClassHashAt",
//!   "params": {
//!     "block_id": "latest",
//!     "contract_address": "0x123..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getClassAt`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_class_at`]
//!
//! Returns the contract class deployed at a given address.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getClassAt",
//!   "params": {
//!     "block_id": "latest",
//!     "contract_address": "0x123..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getBlockTransactionCount`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_block_transaction_count`]
//!
//! Returns the number of transactions in a block.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getBlockTransactionCount",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_call`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::call`]
//!
//! Executes a function call without creating a transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_call",
//!   "params": {
//!     "request": {
//!       "contract_address": "0x123...",
//!       "entry_point_selector": "0x456...",
//!       "calldata": ["0x789..."]
//!     },
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_estimateFee`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::estimate_fee`]
//!
//! Estimates the fee for a given transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_estimateFee",
//!   "params": {
//!     "request": [/* transaction objects */],
//!     "simulation_flags": [],
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_estimateMessageFee`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::estimate_message_fee`]
//!
//! Estimates the fee for an L1 to L2 message.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_estimateMessageFee",
//!   "params": {
//!     "message": {/* L1 message object */},
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_chainId`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::chain_id`]
//!
//! Returns the chain ID of the network.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_chainId",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_syncing`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::syncing`]
//!
//! Returns synchronization status of the node.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_syncing",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getEvents`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_events`]
//!
//! Returns events matching the provided filter.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getEvents",
//!   "params": {
//!     "filter": {
//!       "from_block": "0x0",
//!       "to_block": "latest",
//!       "address": "0x123...",
//!       "keys": [["0x456..."]]
//!     }
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getNonce`
//!
//! [`versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::get_nonce`]
//!
//! Returns the nonce of a contract account at the given Block ID.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getNonce",
//!   "params": {
//!     "block_id": "latest",
//!     "contract_address": "0x123..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getCompiledCasm` (v0.8.0+)
//!
//! [`versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server::get_compiled_casm`]
//!
//! Returns the compiled CASM code for a Sierra contract class.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getCompiledCasm",
//!   "params": {
//!     "class_hash": "0xabc..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_getStorageProof` (v0.8.0+)
//!
//! [`versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server::get_storage_proof`]
//!
//! Returns merkle proof for storage values.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_getStorageProof",
//!   "params": {
//!     "block_id": "latest",
//!     "contract_addresses": ["0x123..."],
//!     "contracts_storage_keys": [
//!       {
//!         "contract_address": "0x123...",
//!         "storage_keys": ["0x456..."]
//!       }
//!     ]
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_traceTransaction`
//!
//! [`versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::trace_transaction`]
//!
//! Returns the execution trace of a transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_traceTransaction",
//!   "params": {
//!     "transaction_hash": "0x789..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_simulateTransactions`
//!
//! [`versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::simulate_transactions`]
//!
//! Simulates transactions without executing them on-chain.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_simulateTransactions",
//!   "params": {
//!     "block_id": "latest",
//!     "transactions": [/* transaction objects */],
//!     "simulation_flags": []
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_traceBlockTransactions`
//!
//! [`versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::trace_block_transactions`]
//!
//! Returns execution traces for all transactions in a block.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_traceBlockTransactions",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### Write Methods
//!
//! These methods submit transactions to be included in the blockchain. Note that write methods
//! are forwarded to the sequencer and are not executed directly by Madara unless it is in block
//! production mode.
//!
//! #### `starknet_addInvokeTransaction`
//!
//! [`versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::add_invoke_transaction`]
//!
//! Submits an invoke transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_addInvokeTransaction",
//!   "params": {
//!     "invoke_transaction": {/* transaction object */}
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_addDeclareTransaction`
//!
//! [`versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::add_declare_transaction`]
//!
//! Submits a declare transaction to register a new contract class.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_addDeclareTransaction",
//!   "params": {
//!     "declare_transaction": {/* transaction object */}
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_addDeployAccountTransaction`
//!
//! [`versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::add_deploy_account_transaction`]
//!
//! Submits a deploy account transaction.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_addDeployAccountTransaction",
//!   "params": {
//!     "deploy_account_transaction": {/* transaction object */}
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### WebSocket Methods (v0.10.0+)
//!
//! WebSocket methods enable real-time subscriptions to blockchain events. These methods are
//! accessible through the same port as HTTP RPC methods. WebSocket subscriptions are exposed only
//! for RPC v0.10.0 and newer.
//!
//! #### `starknet_subscribeNewHeads`
//!
//! [`versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::subscribe_new_heads`]
//!
//! Creates a subscription for new block headers.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_subscribeNewHeads",
//!   "params": {
//!     "block_id": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_subscribeEvents`
//!
//! [`versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::subscribe_events`]
//!
//! Creates a subscription for contract events.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_subscribeEvents",
//!   "params": {
//!     "from_address": "0x123...",
//!     "keys": [["0x456..."]],
//!     "block": "latest"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### `starknet_subscribeTransactionStatus`
//!
//! [`versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::subscribe_transaction_status`]
//!
//! Creates a subscription for transaction status updates.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_subscribeTransactionStatus",
//!   "params": {
//!     "transaction_hash": "0x789..."
//!   },
//!   "id": 1
//! }
//! ```
//!
//! #### Transaction Stream Subscriptions
//!
//! Madara supports transaction-stream methods for v0.10.0 and newer:
//!
//! - `v0.10.0`: [`versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::subscribe_new_transactions`]
//! - `v0.10.2`: [`versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server::subscribe_new_transactions`]
//!
//! Receipt streaming is exposed through:
//!
//! - [`versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::subscribe_new_transaction_receipts`]
//! - [`versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server::subscribe_new_transaction_receipts`]
//!
//! #### `starknet_unsubscribe`
//!
//! Closes an active WebSocket subscription.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_unsubscribe",
//!   "params": {
//!     "subscription_id": 1
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ## Madara-specific Admin Methods
//!
//! Madara extends the standard Starknet RPC with custom administrative methods. These are
//! exposed on a separate port **9943** by default (configurable via `--rpc-admin-port`) and
//! are restricted to localhost unless explicitly exposed via `--rpc-admin-external`.
//!
//! Be weary when using admin methods as they provide privileged access to node operations. Never
//! expose these endpoints publicly without proper authentication and authorization mechanisms.
//! Madara does not perform authorization checks on these methods.
//!
//! ### Write Methods
//!
//! #### `madara_addDeclareV0Transaction`
//!
//! [`versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server::add_declare_v0_transaction`]
//!
//! Adds a legacy Declare V0 transaction to the state. This method is specific to Madara and
//! allows submission of older transaction formats.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "madara_addDeclareV0Transaction",
//!   "params": {
//!     "declare_transaction": {/* legacy transaction object */}
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### Status Methods
//!
//! #### `madara_ping`
//!
//! [`versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::ping`]
//!
//! Returns the current Unix timestamp, useful for checking node responsiveness.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "madara_ping",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `madara_shutdown`
//!
//! [`versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::shutdown`]
//!
//! Gracefully shuts down the running node.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "madara_shutdown",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! #### `madara_service`
//!
//! [`versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server::service`]
//!
//! Manages the status of node services, allowing starting, stopping, or restarting specific
//! components.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "madara_service",
//!   "params": {
//!     "service": "sync",
//!     "action": "restart"
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### WebSocket Methods
//!
//! #### `madara_pulse`
//!
//! [`versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::pulse`]
//!
//! Establishes a WebSocket connection that periodically sends heartbeat signals to confirm
//! the node is alive. Useful for monitoring and health checks.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "madara_pulse",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! ## Special Methods
//!
//! #### `rpc_methods`
//!
//! Returns a list of all available RPC methods on the current endpoint. This is useful for
//! discovering which methods are supported by a particular node configuration.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "rpc_methods",
//!   "params": [],
//!   "id": 1
//! }
//! ```
//!
//! [Starknet RPC Specs]: https://github.com/starkware-libs/starknet-specs

#[cfg(test)]
pub mod test_utils;
pub mod utils;
pub mod versions;

mod block_id;
mod constants;
mod errors;
mod metrics;
mod types;

use jsonrpsee::RpcModule;
use mc_db::MadaraBackend;
use mc_mempool::{
    Mempool, PreConfirmationStatus, TransactionStatus as MempoolTransactionStatus, WatchTransactionStatus,
};
use mc_submit_tx::{SubmitTransaction, TransactionLookup};
use mp_transactions::{validated::ValidatedTransaction, Transaction};
use mp_utils::service::ServiceContext;
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicU64, atomic::Ordering, Arc},
    time::Instant,
};

pub use errors::{StarknetRpcApiError, StarknetRpcResult};

#[derive(Debug, Default)]
pub struct StarknetSubscriptionIdProvider {
    next: AtomicU64,
}

impl jsonrpsee::server::IdProvider for StarknetSubscriptionIdProvider {
    fn next_id(&self) -> jsonrpsee::types::SubscriptionId<'static> {
        self.next.fetch_add(1, Ordering::Relaxed).to_string().into()
    }
}

/// Limits to the storage proof endpoint.
#[derive(Clone, Debug)]
pub struct StorageProofConfig {
    /// Max keys that cna be used in a storage proof.
    pub max_keys: usize,
    /// Max tries that can be used in a storage proof.
    pub max_tries: usize,
    /// How many blocks in the past can we get a storage proof for.
    pub max_distance: u64,
}

impl Default for StorageProofConfig {
    fn default() -> Self {
        Self { max_keys: 1024, max_tries: 5, max_distance: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatusSnapshot {
    Received,
    Candidate,
    PreConfirmed,
    AcceptedOnL2,
    AcceptedOnL1,
}

pub enum TxStatusWatchUpdate {
    Status(Option<TxStatusSnapshot>),
    Closed,
}

pub trait TxStatusWatch: Send {
    fn take_current(&mut self) -> Option<TxStatusSnapshot>;
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = TxStatusWatchUpdate> + Send + '_>>;
}

pub trait TxStatusWatcher: Send + Sync {
    fn watch_transaction_status(&self, transaction_hash: mp_convert::Felt) -> Option<Box<dyn TxStatusWatch + Send>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTransactionsWatchError {
    Lagged,
}

pub type NewTransactionsWatchOutput = Result<Option<Arc<ValidatedTransaction>>, NewTransactionsWatchError>;
pub type NewTransactionsWatchFuture<'a> = Pin<Box<dyn Future<Output = NewTransactionsWatchOutput> + Send + 'a>>;

pub trait NewTransactionsWatch: Send {
    fn recv(&mut self) -> NewTransactionsWatchFuture<'_>;
}

pub trait NewTransactionsWatcher: Send + Sync {
    fn watch_new_transactions(&self) -> Option<Box<dyn NewTransactionsWatch + Send>>;
}

pub(crate) fn normalize_sender_address_filter(
    sender_address: Option<Vec<starknet_types_core::felt::Felt>>,
) -> Option<HashSet<starknet_types_core::felt::Felt>> {
    sender_address.and_then(|addresses| {
        let addresses = addresses.into_iter().collect::<HashSet<_>>();
        (!addresses.is_empty()).then_some(addresses)
    })
}

pub(crate) fn transaction_matches_sender(
    transaction: &Transaction,
    sender_address: Option<&HashSet<starknet_types_core::felt::Felt>>,
) -> bool {
    let Some(sender_address) = sender_address else {
        return true;
    };
    if sender_address.is_empty() {
        return true;
    }

    match transaction {
        Transaction::Invoke(inner) => sender_address.contains(inner.sender_address()),
        Transaction::L1Handler(inner) => sender_address.contains(&inner.contract_address),
        Transaction::Declare(inner) => sender_address.contains(inner.sender_address()),
        Transaction::Deploy(inner) => sender_address.contains(&inner.calculate_contract_address()),
        Transaction::DeployAccount(inner) => sender_address.contains(&inner.calculate_contract_address()),
    }
}

fn tx_status_snapshot(status: Option<MempoolTransactionStatus>) -> Option<TxStatusSnapshot> {
    match status {
        Some(MempoolTransactionStatus::Preconfirmed(PreConfirmationStatus::Received(_))) => {
            Some(TxStatusSnapshot::Received)
        }
        Some(MempoolTransactionStatus::Preconfirmed(PreConfirmationStatus::Candidate { .. })) => {
            Some(TxStatusSnapshot::Candidate)
        }
        Some(MempoolTransactionStatus::Preconfirmed(PreConfirmationStatus::Executed { .. })) => {
            Some(TxStatusSnapshot::PreConfirmed)
        }
        Some(MempoolTransactionStatus::Confirmed { is_on_l1, .. }) => {
            Some(if is_on_l1 { TxStatusSnapshot::AcceptedOnL1 } else { TxStatusSnapshot::AcceptedOnL2 })
        }
        None => None,
    }
}

impl<D: mc_db::MadaraStorageRead> TxStatusWatch for WatchTransactionStatus<D> {
    fn take_current(&mut self) -> Option<TxStatusSnapshot> {
        let snapshot = tx_status_snapshot(WatchTransactionStatus::current(self).clone());
        WatchTransactionStatus::refresh(self);
        snapshot
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = TxStatusWatchUpdate> + Send + '_>> {
        Box::pin(async move {
            WatchTransactionStatus::recv(self)
                .await
                .map(|status| TxStatusWatchUpdate::Status(tx_status_snapshot(status.clone())))
                .unwrap_or(TxStatusWatchUpdate::Closed)
        })
    }
}

impl<D: mc_db::MadaraStorageRead> TxStatusWatcher for Mempool<D> {
    fn watch_transaction_status(&self, transaction_hash: mp_convert::Felt) -> Option<Box<dyn TxStatusWatch + Send>> {
        let watch = self.watch_transaction_status(transaction_hash).ok()?;
        Some(Box::new(watch))
    }
}

struct BroadcastNewTransactionsWatch {
    receiver: tokio::sync::broadcast::Receiver<Arc<ValidatedTransaction>>,
}

impl NewTransactionsWatch for BroadcastNewTransactionsWatch {
    fn recv(&mut self) -> NewTransactionsWatchFuture<'_> {
        Box::pin(async move {
            match self.receiver.recv().await {
                Ok(tx) => Ok(Some(tx)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Err(NewTransactionsWatchError::Lagged),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => Ok(None),
            }
        })
    }
}

impl<D: mc_db::MadaraStorageRead + mc_db::MadaraStorageWrite> NewTransactionsWatcher for Mempool<D> {
    fn watch_new_transactions(&self) -> Option<Box<dyn NewTransactionsWatch + Send>> {
        Some(Box::new(BroadcastNewTransactionsWatch { receiver: self.subscribe_new_transactions() }))
    }
}

/// A Starknet RPC server for Madara
#[derive(Clone)]
pub struct Starknet {
    backend: Arc<MadaraBackend>,
    pub(crate) mempool: Option<Arc<Mempool>>,
    ws_handles: Arc<WsSubscribeHandles>,
    pub(crate) pre_v0_9_preconfirmed_as_pending: bool,
    pub(crate) transaction_submitter: Arc<dyn SubmitTransaction>,
    pub(crate) transaction_lookup: Arc<dyn TransactionLookup>,
    pub(crate) tx_status_watcher: Option<Arc<dyn TxStatusWatcher>>,
    pub(crate) new_transactions_watcher: Option<Arc<dyn NewTransactionsWatcher>>,
    storage_proof_config: StorageProofConfig,
    pub(crate) block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
    pub ctx: ServiceContext,
    pub(crate) rpc_unsafe_enabled: bool,
}

impl Starknet {
    pub fn new(
        backend: Arc<MadaraBackend>,
        transaction_submitter: Arc<dyn SubmitTransaction>,
        transaction_lookup: Arc<dyn TransactionLookup>,
        storage_proof_config: StorageProofConfig,
        block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
        ctx: ServiceContext,
    ) -> Self {
        let ws_handles = Arc::new(WsSubscribeHandles::new());
        Self {
            backend,
            mempool: None,
            ws_handles,
            transaction_submitter,
            transaction_lookup,
            tx_status_watcher: None,
            new_transactions_watcher: None,
            storage_proof_config,
            block_prod_handle,
            ctx,
            pre_v0_9_preconfirmed_as_pending: false,
            rpc_unsafe_enabled: false,
        }
    }

    pub fn set_pre_v0_9_preconfirmed_as_pending(&mut self, value: bool) {
        self.pre_v0_9_preconfirmed_as_pending = value;
    }

    pub fn set_rpc_unsafe_enabled(&mut self, value: bool) {
        self.rpc_unsafe_enabled = value;
    }

    pub fn set_tx_status_watcher(&mut self, watcher: Option<Arc<dyn TxStatusWatcher>>) {
        self.tx_status_watcher = watcher;
    }

    pub fn set_new_transactions_watcher(&mut self, watcher: Option<Arc<dyn NewTransactionsWatcher>>) {
        self.new_transactions_watcher = watcher;
    }

    pub fn set_mempool(&mut self, mempool: Arc<Mempool>) {
        self.mempool = Some(mempool);
    }

    #[cfg(test)]
    pub(crate) fn active_ws_subscription_count(&self) -> usize {
        self.ws_handles.handles.len()
    }
}

/// Returns the RpcModule merged with all the supported RPC versions.
pub fn rpc_api_user(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetTraceRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetWriteRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetTraceRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_10_0::StarknetReadRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetWriteRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetTraceRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_10_2::StarknetReadRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_2::StarknetWriteRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_2::StarknetTraceRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

pub fn rpc_api_admin(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    if starknet.rpc_unsafe_enabled {
        rpc_api.merge(versions::admin::v0_1_0::MadaraMempoolRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    }
    rpc_api.merge(versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraReadRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

struct WsSubscriptionHandle {
    cancelled: tokio::sync::watch::Sender<bool>,
}

impl WsSubscriptionHandle {
    fn new() -> (Self, tokio::sync::watch::Receiver<bool>) {
        let (cancelled, receiver) = tokio::sync::watch::channel(false);
        (Self { cancelled }, receiver)
    }

    fn cancel(&self) {
        let _ = self.cancelled.send(true);
    }

    #[cfg(test)]
    async fn cancelled(&self) {
        let mut cancelled = self.cancelled.subscribe();
        while !*cancelled.borrow_and_update() {
            if cancelled.changed().await.is_err() {
                return;
            }
        }
    }
}

pub(crate) struct WsSubscribeHandles {
    /// Keeps track of all ws connection handles.
    ///
    /// This can be used to request the closure of a ws connection.
    ///
    /// ## Preventing Leaks
    ///
    /// Stale handles are removed each time a subscription is dropped to keep the backing map from
    /// growing to an unbounded size. Note that there is no hard upper limit on the size of the map,
    /// other than those set in the RPC middleware, but at least this way we clean up connections on
    /// close.
    ///
    /// ## Thread Safety
    ///
    /// From the [DashMap] docs:
    ///
    /// > Documentation mentioning locking behaviour acts in the reference frame of the calling
    /// > thread. This means that it is safe to ignore it across multiple threads.
    ///
    /// And from [DashMap::entry]:
    ///
    /// > Locking behaviour: May deadlock if called when holding any sort of reference into the map.
    ///
    /// This is fine in our case as we do not maintain references to a map in the same thread while
    /// mutating it and instead operate directly on-value by sharing the map inside an [Arc].
    ///
    /// [DashMap]: dashmap::DashMap
    /// [DashMap::entry]: dashmap::DashMap::entry
    /// [Arc]: std::sync::Arc
    handles: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<WsSubscriptionHandle>>>,
    counts_by_method: std::sync::Arc<dashmap::DashMap<&'static str, u64>>,
}

impl WsSubscribeHandles {
    pub fn new() -> Self {
        Self {
            handles: std::sync::Arc::new(dashmap::DashMap::new()),
            counts_by_method: std::sync::Arc::new(dashmap::DashMap::new()),
        }
    }

    // FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
    #[allow(unused)]
    pub async fn subscription_register(
        &self,
        id: jsonrpsee::types::SubscriptionId<'static>,
        method: &'static str,
    ) -> WsSubscriptionGuard {
        let id = match id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(id) => {
                id.parse().expect("Starknet subscription ids should be numeric strings")
            }
        };

        let (handle, cancelled) = WsSubscriptionHandle::new();
        let handle = std::sync::Arc::new(handle);
        let map = std::sync::Arc::clone(&self.handles);

        self.handles.insert(id, std::sync::Arc::clone(&handle));
        let method_count = self.increment_method_count(method);
        let metrics = crate::metrics::ws_metrics();
        metrics.record_subscription_opened(method);
        metrics.record_active_subscriptions(self.handles.len() as u64);
        metrics.record_active_subscriptions_for_method(method, method_count);
        tracing::info!(
            "WS subscription opened: method={} subscription_id={} active_subscriptions={} active_method_subscriptions={}",
            method,
            id,
            self.handles.len(),
            method_count
        );

        WsSubscriptionGuard {
            id,
            method,
            opened_at: Instant::now(),
            _handle: handle,
            cancelled,
            map,
            counts_by_method: std::sync::Arc::clone(&self.counts_by_method),
        }
    }

    pub async fn subscription_close(&self, id: u64) -> bool {
        if let Some((_, handle)) = self.handles.remove(&id) {
            tracing::info!("WS subscription close requested: subscription_id={} reason=starknet_unsubscribe", id);
            handle.cancel();
            true
        } else {
            tracing::warn!(
                "WS subscription close requested for unknown subscription: subscription_id={} reason=starknet_unsubscribe",
                id
            );
            false
        }
    }

    fn increment_method_count(&self, method: &'static str) -> u64 {
        let mut count = self.counts_by_method.entry(method).or_insert(0);
        *count += 1;
        *count
    }
}

pub(crate) struct WsSubscriptionGuard {
    id: u64,
    method: &'static str,
    opened_at: Instant,
    // Keep the registered handle alive until this guard is dropped.
    _handle: std::sync::Arc<WsSubscriptionHandle>,
    cancelled: tokio::sync::watch::Receiver<bool>,
    map: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<WsSubscriptionHandle>>>,
    counts_by_method: std::sync::Arc<dashmap::DashMap<&'static str, u64>>,
}

impl WsSubscriptionGuard {
    pub async fn cancelled(&self) {
        let mut cancelled = self.cancelled.clone();
        while !*cancelled.borrow_and_update() {
            if cancelled.changed().await.is_err() {
                return;
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }
}

pub(crate) async fn close_ws_subscription(
    starknet: &Starknet,
    subscription_id: jsonrpsee::types::SubscriptionId<'_>,
    parse_error_context: &'static str,
) -> Result<(), errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    let subscription_id = match subscription_id {
        jsonrpsee::types::SubscriptionId::Num(id) => id,
        jsonrpsee::types::SubscriptionId::Str(id) => id.parse().or_internal_server_error(parse_error_context)?,
    };

    let _ = starknet.ws_handles.subscription_close(subscription_id).await;
    Ok(())
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum LiveConfirmedHeadResolution {
    Block(Box<mp_block::MadaraBlockInfo>),
    Reorg(mc_db::ReorgNotification),
    RetryBackfill,
}

pub(crate) fn try_recv_live_reorg(
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
    missed_reorg_error: impl FnOnce() -> errors::StarknetWsApiError,
) -> Result<Option<mc_db::ReorgNotification>, errors::StarknetWsApiError> {
    match reorgs.try_recv() {
        Ok(reorg) => Ok(Some(reorg)),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => Err(missed_reorg_error()),
        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => Err(errors::StarknetWsApiError::Internal),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => Ok(None),
    }
}

pub(crate) fn resolve_live_confirmed_head(
    backend: &std::sync::Arc<mc_db::MadaraBackend>,
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
    next_block_n: u64,
    missed_reorg_error: impl FnOnce() -> errors::StarknetWsApiError,
) -> Result<LiveConfirmedHeadResolution, errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    if let Some(reorg) = try_recv_live_reorg(reorgs, missed_reorg_error)? {
        return Ok(LiveConfirmedHeadResolution::Reorg(reorg));
    }

    let Some(block_view) = backend.block_view_on_confirmed(next_block_n) else {
        return Ok(LiveConfirmedHeadResolution::RetryBackfill);
    };
    let block_info = block_view
        .get_block_info()
        .or_else_internal_server_error(|| format!("Failed to retrieve block info for block {next_block_n}"))?;

    if block_info.header.block_number != next_block_n {
        let err = format!("Retrieved mismatched block {}, expected {next_block_n}", block_info.header.block_number);
        return Err(errors::StarknetWsApiError::internal_server_error(err));
    }

    Ok(LiveConfirmedHeadResolution::Block(Box::new(block_info)))
}

impl Drop for WsSubscriptionGuard {
    fn drop(&mut self) {
        self.map.remove(&self.id);
        let method_count = if let Some(mut count) = self.counts_by_method.get_mut(self.method) {
            *count = count.saturating_sub(1);
            *count
        } else {
            0
        };
        let age = self.opened_at.elapsed();
        let metrics = crate::metrics::ws_metrics();
        metrics.record_subscription_closed(self.method);
        metrics.record_subscription_duration(self.method, age.as_secs_f64());
        metrics.record_active_subscriptions(self.map.len() as u64);
        metrics.record_active_subscriptions_for_method(self.method, method_count);
        tracing::info!(
            "WS subscription closed: method={} subscription_id={} age_secs={} active_subscriptions={} active_method_subscriptions={}",
            self.method,
            self.id,
            age.as_secs(),
            self.map.len(),
            method_count
        );
    }
}

#[cfg(test)]
mod test {
    use super::{
        normalize_sender_address_filter, resolve_live_confirmed_head, LiveConfirmedHeadResolution, WsSubscriptionHandle,
    };
    use crate::{errors::StarknetWsApiError, test_utils::rpc_test_setup};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
    use starknet_types_core::felt::Felt;
    use std::{collections::HashSet, sync::Arc};

    fn add_block_at(backend: &Arc<mc_db::MadaraBackend>, n: u64) -> Felt {
        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: n, ..Default::default() },
                    state_diff: mp_state_update::StateDiff::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .expect("Storing block")
            .block_hash
    }

    #[test]
    fn resolve_live_confirmed_head_returns_pending_reorg_before_reading_db() {
        let (backend, _rpc) = rpc_test_setup();
        let block_0_hash = add_block_at(&backend, 0);
        let block_1_hash = add_block_at(&backend, 1);
        let mut reorgs = backend.subscribe_reorgs();

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        match resolve_live_confirmed_head(&backend, &mut reorgs, 1, || StarknetWsApiError::Internal)
            .expect("Reorg resolution should succeed")
        {
            LiveConfirmedHeadResolution::Reorg(reorg) => {
                assert_eq!(reorg.first_reverted_block_n, 1);
                assert_eq!(reorg.first_reverted_block_hash, block_1_hash);
            }
            LiveConfirmedHeadResolution::Block(_) => panic!("Expected queued reorg before block read"),
            LiveConfirmedHeadResolution::RetryBackfill => panic!("Expected queued reorg before backfill retry"),
        }
    }

    #[test]
    fn resolve_live_confirmed_head_retries_backfill_when_block_is_missing() {
        let (backend, _rpc) = rpc_test_setup();
        let mut reorgs = backend.subscribe_reorgs();

        match resolve_live_confirmed_head(&backend, &mut reorgs, 0, || StarknetWsApiError::Internal)
            .expect("Missing block should not error")
        {
            LiveConfirmedHeadResolution::RetryBackfill => {}
            LiveConfirmedHeadResolution::Block(_) => panic!("Expected missing block to retry backfill"),
            LiveConfirmedHeadResolution::Reorg(_) => panic!("Expected missing block without reorg to retry backfill"),
        }
    }

    #[test]
    fn normalize_sender_address_filter_treats_empty_as_unfiltered() {
        assert_eq!(normalize_sender_address_filter(None), None);
        assert_eq!(normalize_sender_address_filter(Some(vec![])), None);
        assert_eq!(
            normalize_sender_address_filter(Some(vec![Felt::ONE, Felt::ONE, Felt::TWO])),
            Some(HashSet::from([Felt::ONE, Felt::TWO]))
        );
    }

    #[tokio::test]
    async fn ws_subscription_handle_cancel_wakes_all_waiters() {
        let (handle, _cancelled) = WsSubscriptionHandle::new();
        let handle = Arc::new(handle);
        let handle_1 = Arc::clone(&handle);
        let handle_2 = Arc::clone(&handle);

        let waiter_1 = tokio::spawn(async move { handle_1.cancelled().await });
        let waiter_2 = tokio::spawn(async move { handle_2.cancelled().await });

        tokio::task::yield_now().await;
        handle.cancel();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            waiter_1.await.expect("First waiter should complete");
            waiter_2.await.expect("Second waiter should complete");
        })
        .await
        .expect("Cancellation should wake all waiters");
    }

    #[tokio::test]
    async fn ws_subscription_handle_cancelled_returns_immediately_after_cancel() {
        let (handle, _cancelled) = WsSubscriptionHandle::new();
        handle.cancel();

        tokio::time::timeout(std::time::Duration::from_secs(1), handle.cancelled())
            .await
            .expect("Cancelled handle should resolve immediately");
    }
}
