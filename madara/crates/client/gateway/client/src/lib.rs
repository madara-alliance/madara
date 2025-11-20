//! # Feeder Gateway Client
//!
//! ## Role and Purpose
//!
//! This crate provides the **client implementation** for interacting with feeder gateway and gateway
//! endpoints. It is used by:
//!
//! - **Full nodes** during block synchronization to fetch blocks, state updates, and classes from sequencer
//! - **Full nodes** to forward user transactions to sequencer nodes for block production
//!
//! If you're looking for the server-side implementation that **serves** these endpoints, see the
//! gateway server crate under `gateway/server`.
//!
//! # Feeder Gateway
//!
//! The **Feeder Gateway**, which you will sometimes see referred to as _fgw_, is an unspecified
//! endpoint of a Starknet node. It is used for synchronization during block sync from a centralized
//! source, predating Starknet's upgrade to p2p block sync.
//!
//! Despite this, the fgw has never been properly specified. Parity with mainnet implementation is
//! done on a best-effort basis and differences in edge cases are highly likely.
//!
//! This crate implements the **client-side** methods and deserialization logic to connect to and
//! request data from a fgw endpoint during block sync. The client handles connection management,
//! request building, and response deserialization. For the server implementation that serves these
//! requests, check out the fgw server under `gateway/server`.
//!
//! ## Available endpoints
//!
//! Below is a list of endpoints made available to the fgw. Keep in mind that the actual
//! implementation is subject to subtle yet meaningful differences between nodes, and this includes
//! the official Starknet fgw endpoints.
//!
//! ### `get_block`
//!
//! Fetches a block at a given block id from a remote feeder gateway node. Use this during block
//! synchronization to retrieve block data. The optional `headerOnly` field allows you to retrieve
//! only the block header, which can optimize bandwidth when you only need header information.
//!
//! ```json5
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_block",
//!   "params": {
//!     "blockNumber": 123,
//!     // Optional - set to true to fetch only header
//!     // "headerOnly": true,
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_state_update`
//!
//! Fetches the state update at a given block id from a remote feeder gateway node. State updates
//! contain storage diffs, deployed contracts, nonce updates, and declared classes. The optional
//! `includeBlock` field allows you to retrieve both the state update and the block in a single
//! request, which is an optimization commonly used during sync since these are typically needed
//! together.
//!
//! **Use case**: Called by full nodes during the sync process. Setting `includeBlock: true`
//! reduces the number of round trips needed during synchronization.
//!
//! **Note**: When using `includeBlock`, the block id must be specified as a block number (height).
//!
//! **Example request**:
//! ```json5
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_state_update",
//!   "params": {
//!     "blockNumber": 123,
//!     // Optional - fetch block and state update together
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
//!     "blockNumber": 123,
//!   },
//!   "id": 1
//! }
//! ```
//!
//! ### `get_class_by_hash`
//!
//! Fetches a contract class definition at the given `classHash` and block id. Returns either a
//! compressed [`legacy`] class or a flattened [`sierra`] class depending on the class type.
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
//! Fetches a compiled (CASM) representation of a [`sierra`] class at the given `classHash` and
//! block id. This endpoint does **not** support [`legacy`] classes - the given `classHash` must
//! point to a [`sierra`] class.
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
//! Fetches the contract addresses of system contracts from the remote node. This includes:
//! - **ETH token** (parent fee token)
//! - **STRK token** (native fee token)
//! - **Starknet core contract** address on L1 (e.g., Ethereum)
//! - **GpsStatementVerifier** contract address on L1
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
//! Fetches a signed commitment to the block hash at a given block id. The signature is created
//! using the remote node's unique identifying keypair.
//!
//! **Use case**: Called to verify the authenticity of blocks from the remote node. Note that
//! signatures cannot be obtained for the pending block.
//!
//! **Example request**:
//! ```json5
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
//! Fetches the public key associated with the remote node's signing keypair.
//!
//! **Use case**: Called to obtain the public key needed to verify block signatures obtained from
//! `get_signature`.
//!
//! **Example request**:
//! ```json5
//! {
//!   "jsonrpc": "2.0",
//!   "method": "get_public_key",
//!   "id": 1
//! }
//! ```
//!
//! # Gateway Client
//!
//! The **Gateway**, which you will sometimes see referred to as _gw_ (but still most often as just
//! _gateway_), is an unspecified endpoint of a Starknet node similar to the [fgw]. It is used by
//! full nodes to forward transactions to sequencer nodes that are capable of producing blocks.
//!
//! Historically, this is due to the fact that Starknet used to have a single sequencer responsible
//! for state production. In practice, this architecture is still useful for load balancing: ingress
//! nodes can receive and validate user transactions before forwarding them to sequencer nodes. This
//! separation allows validation and block production to occur on different machines, avoiding
//! bottlenecks where validation workload affects block production and therefore chain liveness.
//!
//! This crate implements the **client-side** methods for connecting to and submitting transactions
//! to a gateway endpoint. The client handles transaction serialization, request building, and
//! response handling. For the server implementation, check out the gateway server under
//! `gateway/server`.
//!
//! ## Transaction Forwarding Workflow
//!
//! 1. User submits transaction to a full node (via RPC)
//! 2. Full node validates the transaction
//! 3. Full node uses this client to forward the transaction to a sequencer node
//! 4. Sequencer node receives transaction via gateway server
//! 5. Sequencer includes transaction in block production
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
//! Submits user transactions to a sequencer node for inclusion in a block. This is the primary
//! method for forwarding [declare transactions], [deploy account transactions], and
//! [invoke transactions] from a full node to a sequencer.
//!
//! **Use case**: Called by full nodes after validating user-submitted transactions. The full node
//! acts as an ingress point, validates the transaction, and then forwards it to a sequencer for
//! block production.
//!
//! **Example request**:
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
//! Submits pre-validated transactions directly to a sequencer, **skipping validation**. This is a
//! trusted endpoint that should only be used in specific architectures where validation nodes and
//! sequencer nodes are under the same operational control.
//!
//! **Use case**: In a split architecture where dedicated validator nodes verify transactions and
//! forward them to sequencer nodes, this endpoint allows the sequencer to skip redundant validation
//! work. The validator node calls this endpoint after performing full validation.
//!
//! **Security Note**: This endpoint requires the sequencer to trust the calling node completely.
//! It should only be enabled when connecting to trusted validator nodes, as invalid transactions
//! submitted through this endpoint will bypass all safety checks.
//!
//! **Example request**:
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
//! **Configuration**: The sequencer must explicitly enable this endpoint via CLI flags. It is
//! disabled by default for security reasons.
//!
//! # Error Handling
//!
//! The client handles various error conditions:
//! - **Network errors**: Connection failures, timeouts
//! - **Deserialization errors**: Invalid responses from server
//! - **Protocol errors**: Server returns error codes
//!
//! Callers should implement appropriate retry logic for transient network errors.
//!
//! # Connection Management
//!
//! The client maintains HTTP(S) connections to the gateway endpoint. Connection pooling and
//! timeout settings can be configured when building the `GatewayProvider`.
//!
//! [`legacy`]: mp_class::ContractClass::Legacy
//! [`sierra`]: mp_class::ContractClass::Sierra
//! [fgw]: #feeder-gateway
//! [declare transactions]: mp_gateway::user_transaction::UserTransaction::Declare
//! [deploy account transactions]: mp_gateway::user_transaction::UserTransaction::DeployAccount
//! [invoke transactions]: mp_gateway::user_transaction::UserTransaction::InvokeFunction

mod builder;
mod health;
mod methods;
mod request_builder;
mod retry;
mod submit_tx;

#[cfg(test)]
mod tests;

pub use mp_rpc::v0_7_1::{BlockId, BlockTag};

pub use builder::GatewayProvider;
pub use health::{start_gateway_health_monitor, GATEWAY_HEALTH};
pub use retry::{RetryConfig, RetryPhase, RetryState};
