//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration.

#![warn(missing_docs)]

mod starknet;

use jsonrpsee::RpcModule;
use mc_rpc::{Starknet, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
pub use sc_rpc_api::DenyUnsafe;
pub use starknet::StarknetDeps;

/// Instantiate all full RPC extensions.
pub fn create_full(starting_block: u64) -> anyhow::Result<RpcModule<()>> {
    let mut module = RpcModule::new(());
    module.merge(StarknetReadRpcApiServer::into_rpc(Starknet::new(starting_block)))?;
    module.merge(StarknetWriteRpcApiServer::into_rpc(Starknet::new(starting_block)))?;
    module.merge(StarknetTraceRpcApiServer::into_rpc(Starknet::new(starting_block)))?;

    Ok(module)
}
