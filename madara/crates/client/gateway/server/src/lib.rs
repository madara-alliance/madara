//! #  Feeder Gateway
//!
//! The **Feeder Gateway**, which you will sometimes see referred to as _fgw_, is an unspecified
//! endpoint of a Starknet node. It is used for synchronization during block sync from a centralized
//! source, predating Starknet's upgrade to p2p block sync.
//!
//! Despite this, the fgw has never been properly specified. Parity with mainnet implementation is
//! done on a best-effort basis and differences in edge cases are highly likely.
//!
//! This crate implements a local fgw. For ways to connect, request and deserialize information from
//! an existing fgw, check out the fgw client under `gateway/client`. Keep in mind that the fgw
//! server is disabled by default and has to be explicitly enabled via the cli on node startup.
//!
//! ## Available endpoints
//!
//! Below is a list of endpoints made available to the fgw. Keep in mind that the actual
//! implementation is subject to subtle yet meaningful differences between nodes, and this includes
//! the official Starknet fgw endpoints.
//!
//! ### `get_block`
//!
//! Returns the block at a given block id from the backend. This endpoint can also be called with
//! an optional `headerOnly` field to retrieve only the header at the specified block.
//!
//! ```json5
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_block",
//!   "params": {
//!     "blockNumber": 123,
//!     // Optional
//!     // "headerOnly": true,
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_state_update`
//!
//! Returns the state update at a given block id from the backend. This endpoint can also be
//! called with an optional `includeBlock` field to retrieve the block at the same block height as
//! the state update. Note that this requires the block id to be specified in terms of a block
//! height to be able fetch both the state update and the block itself with the same id. This can be
//! used to cut down on a request on each call since the state update and the block at a same block
//! height are often used in combination during block sync.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_state_update",
//!   "params": {
//!     "blockNumber": 123,
//!     // Optional
//!     // "includeBlock": true,
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_block_traces`
//!
//! Return a trace of all transactions in a block at a given block id. This is similar to calling
//! the `trace_block_transactions` RPC endpoint.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_block_traces",
//!   "params": {
//!     "b": 123,
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_class_by_hash`
//!
//! Returns a serialized representation of either a compressed [`legacy`] class or a flattened
//! [`sierra`] class at the given `classHash` and block id.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_class_by_hash",
//!   "params": {
//!     "blockNumber": 123,
//!     "classHash": "0xdeadbeef",
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_compiled_class_by_class_hash`
//!
//! Returns a serialized representation of a compiled [`sierra`] class at the given `classHash` and
//! block id. Note that this method does not support [`legacy`] classes and so the given `classHash`
//! _must_ point to a [`sierra`] class.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_compiled_class_by_class_hash",
//!   "params": {
//!     "blockNumber": 123,
//!     "classHash": "0xdeadbeef",
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_contract_addresses`
//!
//! Returns the on-chain address of the `Starknet` core contract and the `GpsStatementVerifier`
//! contract on the layer this node's state is settling on (most likely Ethereum). Note that even if
//! the node is not _producing_ state and is only _synchronizing_ it then this refers to the layer
//! to which that state is being finalized by the block producers the node in synchronizing from.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_contract_addresses",
//!   "id": 1
//! }
//! ```
//!
//! ### `get_signature`
//!
//! Returns a signed commitment to the block hash of a block at the given block id, using the
//! node's unique identifying keypair. Note that a signature _cannot_ be obtained for the pending
//! block.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_signature",
//!   "params": {
//!     "blockNumber": 123,
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_public_key`
//!
//! Returns the public key associated to this node.
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_public_key",
//!   "id": 1
//! }
//! ```
//!
//! # Gateway
//!
//! The **Gateway**, which you will sometimes see referred to as _gw_ (but still most often as just
//! _gateway_), is an unspecified endpoint of a Starknet node similar to the [fgw]. It is used by
//! block producers to receive transactions which are forwarded to it from full nodes which are
//! unable to produce state locally themselves.
//!
//! Historically, this is due to the fact that Starknet used to have a single sequencer responsible
//! for state production. In practice, this is still very handy for load balancing by having some
//! ingress nodes receive user queries and validate them before sending them to a sequencer node for
//! block production. This is because validation, along with block production, both incur a
//! significant amount of computation and so having them take place on different machines can help
//! with avoiding bottlenecks in the validation pipeline affecting block production and therefore the
//! liveliness of the chain.
//!
//! ## Available endpoints
//!
//! Below is a list of endpoints made available to the gateway. Keep in mind that the actual
//! implementation is subject to subtle yet meaningful differences between nodes, and this includes
//! the official Starknet gateway endpoints. Some additional, madara-specific methods have also been
//! added, and it should not be expected that such methods be made available under other node
//! softwares or even work the same if they are.
//!
//! ### `add_transactions`
//!
//! An open endpoint for adding transactions forwarded from other full nodes. This includes
//! [declare transactions], [deploy account transactions] and [invoke transactions].
//!
//! ```json5
//! {
//!   "jsonrpc": "2.0",
//!   "method": "add_transactions",
//!   "params": {
//!      // Transaction info goes here
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `trusted_add_validated_transactions`
//!
//! > **This endpoint is unique to Madara.**
//!
//! A _trusted_ enpoint, **disabled by default**. This allows adding transactions to the state
//! without verifying their validity. As discussed above, this can be useful when splitting up the
//! tasks of validating transactions and adding them to the block production between two nodes.
//!
//! ```json5
//! {
//!   "jsonrpc": "2.0",
//!   "method": "trusted_add_validated_transactions",
//!   "params": {
//!      // Transaction info goes here
//!   },
//!   "id": 1
//! }
//! ```
//!
//! [`legacy`]: mp_class::ContractClass::Legacy
//! [`sierra`]: mp_class::ContractClass::Legacy
//! [fgw]: #feeder-gateway
//! [declare transactions]: mp_gateway::user_transaction::UserTransaction::Declare
//! [deploy account transactions]: mp_gateway::user_transaction::UserTransaction::DeployAccount
//! [invoke transactions]: mp_gateway::user_transaction::UserTransaction::InvokeFunction

mod error;
mod handler;
mod helpers;
mod router;
pub mod service;
