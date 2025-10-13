//! # Feeder Gateway Server
//!
//! ## Role and Purpose
//!
//! This crate provides the **server implementation** for feeder gateway and gateway endpoints.
//! It is used by:
//!
//! - **Sequencer nodes** to serve block data, state updates, and classes to syncing full nodes
//! - **Sequencer nodes** to receive transactions from full nodes for inclusion in blocks
//!
//! The server fetches data from the local backend (database) and serves it to requesting clients.
//! If you're looking for the client-side implementation that **calls** these endpoints, see the
//! gateway client crate under `gateway/client`.
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
//! This crate implements a **server-side** fgw that serves block data to syncing nodes. The server
//! handles incoming requests, fetches data from the local backend/database, and serializes responses.
//! For the client implementation that connects to and requests data from a fgw, check out the fgw
//! client under `gateway/client`.
//!
//! **Note**: The fgw server is **disabled by default** and must be explicitly enabled via CLI flags
//! on node startup.
//!
//! ## Available endpoints
//!
//! Below is a list of endpoints made available to the fgw. Keep in mind that the actual
//! implementation is subject to subtle yet meaningful differences between nodes, and this includes
//! the official Starknet fgw endpoints.
//!
//! ### `get_block`
//!
//! Serves block data at the requested block id from the local backend. The server queries the
//! database for the block and returns it to the requesting client. If the optional `headerOnly`
//! field is set to true, only the block header is returned, reducing response size.
//!
//! **Server behavior**:
//! - Validates the block id parameter
//! - Queries the backend for the block at the specified height or hash
//! - Returns error if block doesn't exist
//! - Serializes and returns either full block or header only
//!
//! **Example request**:
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
//! Serves the state update at the requested block id from the local backend. The server queries
//! storage for the state diff (storage changes, nonce updates, deployed contracts, declared classes)
//! and returns it to the client. The optional `includeBlock` field allows the client to request
//! both the state update and block data in a single response.
//!
//! **Server behavior**:
//! - Validates the block id parameter
//! - Queries the backend for state update at specified block
//! - If `includeBlock` is true and block id is a number, also fetches the block
//! - Returns error if state update doesn't exist
//! - Serializes and returns state update (and optionally the block)
//!
//! **Example request**:
//! ```json5
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
//! Serves execution traces for all transactions in a block at the requested block id. The server
//! queries the backend for stored transaction traces and returns them.
//!
//! **Server behavior**:
//! - Validates the block id parameter
//! - Queries backend for all transaction traces in the specified block
//! - Returns error if block doesn't exist or traces are not available
//! - Serializes and returns the traces
//!
//! **Note**: Trace storage must be enabled in the node configuration for this endpoint to work.
//!
//! **Example request**:
//! ```json5
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
//! Serves the contract class definition for the given `classHash` at the specified block id. The
//! server queries the backend for the class and returns either a compressed [`legacy`] class or a
//! flattened [`sierra`] class depending on the class type.
//!
//! **Server behavior**:
//! - Validates parameters (classHash and block id)
//! - Queries backend for class at the given class hash
//! - Returns error if class doesn't exist or isn't declared at the given block
//! - Serializes and returns the class definition
//!
//! **Example request**:
//! ```json5
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
//! Serves the compiled (CASM) representation of a [`sierra`] class for the given `classHash` at
//! the specified block id. This endpoint does **not** support [`legacy`] classes - the server will
//! return an error if the classHash points to a legacy class.
//!
//! **Server behavior**:
//! - Validates parameters (classHash and block id)
//! - Queries backend for compiled class
//! - Returns error if class doesn't exist, isn't a Sierra class, or isn't declared at given block
//! - Serializes and returns the compiled CASM code
//!
//! **Example request**:
//! ```json5
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
//! Serves the system contract addresses configured for this node. Returns:
//! - **ETH token** (parent fee token) contract address
//! - **STRK token** (native fee token) contract address
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
//! Serves a cryptographic signature over the block hash at the requested block id. The server
//! signs the block hash using the node's private signing key.
//!
//! **Server behavior**:
//! - Validates the block id parameter
//! - Queries backend for block hash at specified block
//! - Returns error if block doesn't exist or is the pending block (cannot sign pending)
//! - Signs the block hash using node's keypair
//! - Returns the signature
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
//! Serves the public key associated with this node's signing keypair. Clients use this to verify
//! signatures obtained from `get_signature`.
//!
//! **Server behavior**:
//! - Reads the public key from node configuration
//! - Returns the public key
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
//! # Gateway Server
//!
//! The **Gateway**, which you will sometimes see referred to as _gw_ (but still most often as just
//! _gateway_), is an unspecified endpoint of a Starknet node similar to the [fgw]. It is used by
//! sequencer nodes to receive transactions from full nodes for inclusion in blocks.
//!
//! Historically, this is due to the fact that Starknet used to have a single sequencer responsible
//! for state production. In practice, this architecture is still useful for load balancing: ingress
//! nodes can receive and validate user transactions before forwarding them to sequencer nodes. This
//! separation allows validation and block production to occur on different machines, avoiding
//! bottlenecks where validation workload affects block production and therefore chain liveness.
//!
//! This crate implements a **server-side** gateway that receives and processes transactions from
//! full nodes. The server validates transactions (unless using the trusted endpoint), adds them to
//! the mempool, and makes them available for block production. For the client implementation that
//! submits transactions to a gateway, check out the gateway client under `gateway/client`.
//!
//! **Note**: The gateway server is **disabled by default** and must be explicitly enabled via CLI
//! flags on node startup.
//!
//! ## Transaction Processing Flow
//!
//! 1. Full node (client) submits transaction via `add_transactions`
//! 2. Gateway server receives and deserializes transaction
//! 3. Server validates transaction (signature, nonce, balance, etc.)
//! 4. Valid transactions are added to the mempool
//! 5. Block production pulls transactions from mempool
//! 6. Server returns transaction hash or error to client
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
//! Receives transactions from full nodes and processes them for block inclusion. The server
//! validates the transaction and adds it to the mempool if valid. This endpoint accepts
//! [declare transactions], [deploy account transactions], and [invoke transactions].
//!
//! **Server behavior**:
//! - Deserializes incoming transaction
//! - Validates transaction signature
//! - Validates nonce and account state
//! - Checks fee payment capabilities
//! - Executes transaction validation logic
//! - Adds valid transaction to mempool
//! - Returns transaction hash on success or error on failure
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
//! > **This endpoint is unique to Madara and must be explicitly enabled via CLI.**
//!
//! Receives pre-validated transactions from trusted validator nodes and adds them directly to the
//! mempool **without performing any validation checks**. This is a trusted endpoint designed for
//! architectures where dedicated validator nodes handle transaction validation separately from
//! block production.
//!
//! **Server behavior**:
//! - Deserializes incoming transaction
//! - **SKIPS all validation** (signature, nonce, balance, etc.)
//! - Directly adds transaction to mempool
//! - Returns transaction hash
//!
//! **Security considerations**:
//! - This endpoint is **disabled by default**
//! - Must be explicitly enabled via CLI flags
//! - Should only be enabled when connecting to trusted validator nodes
//! - No authentication/authorization is performed (consider using network-level restrictions)
//! - Invalid transactions will be added to mempool and may cause block production issues
//!
//! **Use case**: In a split architecture where validation nodes and sequencer nodes are under the
//! same operational control, this eliminates redundant validation work at the sequencer.
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
//! # Configuration and Deployment
//!
//! ## Enabling the Feeder Gateway
//!
//! The feeder gateway server is disabled by default. Enable it with:
//! ```bash
//! madara --feeder-gateway-enable
//! ```
//!
//! ## Enabling the Gateway
//!
//! The gateway server is disabled by default. Enable it with:
//! ```bash
//! madara --gateway-enable
//! ```
//!
//! ## Enabling Trusted Endpoints
//!
//! The `trusted_add_validated_transactions` endpoint requires explicit enabling:
//! ```bash
//! madara --gateway-enable --gateway-enable-trusted
//! ```
//!
//! **Warning**: Only enable trusted endpoints when connecting to validator nodes you control.
//!
//! # Security Considerations
//!
//! - The gateway server should be deployed behind appropriate network security (firewall, rate limiting)
//! - The trusted endpoint should be restricted to specific IP addresses or networks
//! - Consider implementing authentication for trusted endpoints in production
//! - Monitor for unusual transaction patterns or potential DoS attacks
//!
//! [`legacy`]: mp_class::ContractClass::Legacy
//! [`sierra`]: mp_class::ContractClass::Sierra
//! [fgw]: #feeder-gateway
//! [declare transactions]: mp_gateway::user_transaction::UserTransaction::Declare
//! [deploy account transactions]: mp_gateway::user_transaction::UserTransaction::DeployAccount
//! [invoke transactions]: mp_gateway::user_transaction::UserTransaction::InvokeFunction

mod error;
mod handler;
mod helpers;
mod router;
pub mod service;
