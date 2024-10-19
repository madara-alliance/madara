//! Starknet RPC server API implementation
//!
//! It uses the madara client and backend in order to answer queries.

mod constants;
mod macros;
pub mod providers;
#[cfg(test)]
pub mod test_utils;
mod types;
pub mod utils;
pub mod versions;

use jsonrpsee::RpcModule;

use mp_rpc::Starknet;

/// Returns the RpcModule merged with all the supported RPC versions.
pub fn versioned_rpc_api(
    starknet: &Starknet,
    read: bool,
    write: bool,
    trace: bool,
    internal: bool,
) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    merge_rpc_versions!(
        rpc_api, starknet, read, write, trace, internal,
        v0_7_1, // We can add new versions by adding the version module below
                // , v0_8_0 (for example)
    );

    Ok(rpc_api)
}
