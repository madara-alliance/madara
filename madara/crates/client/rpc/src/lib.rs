//! Starknet RPC module. Implements the JSON-RPC server for Madara, providing a standardized
//! interface for interacting with the Starknet node. This module implements the official Starknet
//! JSON-RPC specification along with some Madara-specific extensions.
//!
//! Madara fully supports the Starknet JSON-RPC specification versions `v0.7.1`, `v0.8.1`, `v0.9.0`, and `v0.10.0`, with
//! methods accessible through port **9944** by default (configurable via `--rpc-port`). The RPC
//! server supports both HTTP and WebSocket connections on the same port.
//!
//! ## Version Management
//!
//! RPC methods are versioned to ensure backward compatibility. To access methods from a specific
//! version, append `/rpc/v.../` to your RPC url, where `v...` is your version code. For example:
//!
//! - Default (v0.7.1): `http://localhost:9944/`
//! - Version 0.8.1: `http://localhost:9944/rpc/v0_8_1/`
//! - Version 0.9.0: `http://localhost:9944/rpc/v0_9_0/`
//! - Version 0.10.0: `http://localhost:9944/rpc/v0_10_0/`
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
//! ### WebSocket Methods (v0.8.0+)
//!
//! WebSocket methods enable real-time subscriptions to blockchain events. These methods are
//! accessible through the same port as HTTP RPC methods.
//!
//! #### `starknet_subscribeNewHeads`
//!
//! [`versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server::subscribe_new_heads`]
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
//! [`versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server::subscribe_events`]
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
//! [`versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server::subscribe_transaction_status`]
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
//! #### `starknet_subscribePendingTransactions`
//!
//! [`versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server::subscribe_pending_transactions`]
//!
//! Creates a subscription for pending transactions.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "starknet_subscribePendingTransactions",
//!   "params": {
//!     "transaction_details": false,
//!     "sender_address": ["0x123...", "0x456..."]
//!   },
//!   "id": 1
//! }
//! ```
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
mod types;

use jsonrpsee::RpcModule;
use mc_db::MadaraBackend;
use mc_submit_tx::SubmitTransaction;
use mp_utils::service::ServiceContext;
use std::sync::Arc;

pub use errors::{StarknetRpcApiError, StarknetRpcResult};

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

/// A Starknet RPC server for Madara
#[derive(Clone)]
pub struct Starknet {
    backend: Arc<MadaraBackend>,
    ws_handles: Arc<WsSubscribeHandles>,
    pub(crate) pre_v0_9_preconfirmed_as_pending: bool,
    pub(crate) add_transaction_provider: Arc<dyn SubmitTransaction>,
    storage_proof_config: StorageProofConfig,
    pub(crate) block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
    pub ctx: ServiceContext,
    pub(crate) rpc_unsafe_enabled: bool,
}

impl Starknet {
    pub fn new(
        backend: Arc<MadaraBackend>,
        add_transaction_provider: Arc<dyn SubmitTransaction>,
        storage_proof_config: StorageProofConfig,
        block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
        ctx: ServiceContext,
    ) -> Self {
        let ws_handles = Arc::new(WsSubscribeHandles::new());
        Self {
            backend,
            ws_handles,
            add_transaction_provider,
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
}

/// Returns the RpcModule merged with all the supported RPC versions.
pub fn rpc_api_user(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetTraceRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetWriteRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetWsRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetTraceRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_10_0::StarknetReadRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetWriteRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetTraceRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

pub fn rpc_api_admin(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraReadRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
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
    handles: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<tokio::sync::Notify>>>,
}

impl WsSubscribeHandles {
    pub fn new() -> Self {
        Self { handles: std::sync::Arc::new(dashmap::DashMap::new()) }
    }

    // FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
    #[allow(unused)]
    pub async fn subscription_register(&self, id: jsonrpsee::types::SubscriptionId<'static>) -> WsSubscriptionGuard {
        let id = match id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(_) => {
                unreachable!("Jsonrpsee middleware has been configured to use u64 subscription ids")
            }
        };

        let handle = std::sync::Arc::new(tokio::sync::Notify::new());
        let map = std::sync::Arc::clone(&self.handles);

        self.handles.insert(id, std::sync::Arc::clone(&handle));

        WsSubscriptionGuard { id, handle, map }
    }

    pub async fn subscription_close(&self, id: u64) -> bool {
        if let Some((_, handle)) = self.handles.remove(&id) {
            handle.notify_one();
            true
        } else {
            false
        }
    }
}

pub(crate) struct WsSubscriptionGuard {
    id: u64,
    // FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
    #[allow(unused)]
    handle: std::sync::Arc<tokio::sync::Notify>,
    map: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<tokio::sync::Notify>>>,
}

impl WsSubscriptionGuard {
    // FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
    #[allow(unused)]
    pub async fn cancelled(&self) {
        self.handle.notified().await
    }
}

impl Drop for WsSubscriptionGuard {
    fn drop(&mut self) {
        self.map.remove(&self.id);
    }
}
