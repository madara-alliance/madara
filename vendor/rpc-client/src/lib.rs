//! # RPC Client
//!
//! This crate provides a unified RPC client for interacting with Starknet nodes, supporting both
//! standard Starknet RPC endpoints and Pathfinder-specific extensions.
//!
//! ## Features
//!
//! - **Unified Client**: Single interface for both Starknet and Pathfinder RPC endpoints
//! - **Async Support**: Full async/await support for all operations
//! - **Proof Verification**: Built-in support for storage and class proof verification
//! - **State Reading**: High-level state reading interface for contract interactions
//! - **Error Handling**: Comprehensive error handling with proper error types
//!
//! ## Modules
//!
//! - [`client`] - Main RPC client implementation
//! - [`pathfinder`] - Pathfinder-specific RPC client and proof verification
//! - [`state_reader`] - High-level state reading interface
//! - [`utils`] - Utility functions for async operations
//!
//! ## Usage
//!
//! ```rust
//! use rpc_client::RpcClient;
//! use starknet::core::types::BlockId;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a new RPC client
//!     use starknet_core::types::BlockTag;
//! use starknet_types_core::felt::Felt;
//! let client = RpcClient::new("https://your-starknet-node.com");
//!
//!     // Use standard Starknet RPC
//!     let block = client.starknet_rpc().get_block(BlockId::Tag(BlockTag::Latest)).await?;
//!
//!     // Use Pathfinder-specific RPC
//!     let proof = client.pathfinder_rpc().get_proof(
//!         12345,
//!         Felt::from(12345),
//!         &[Felt::from(12345)]
//!     ).await?;
//!
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod constants;
pub mod error;
pub mod state_reader;
pub mod types;
pub mod utils;

pub use client::RpcClient;
