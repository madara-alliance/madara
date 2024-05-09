use jsonrpsee::core::RpcResult;
use mc_sync::utility::get_config;

use crate::{errors::StarknetRpcApiError, Felt};

/// Return the currently configured chain id.
///
/// This function provides the chain id for the network that the node is connected to. The chain
/// id is a unique identifier that distinguishes between different networks, such as mainnet or
/// testnet.
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// Returns the chain id this node is connected to. The chain id is returned as a specific type,
/// defined by the Starknet protocol, indicating the particular network.
pub fn chain_id() -> RpcResult<Felt> {
    let config = get_config().ok_or_else(|| {
        log::error!("Failed to get config");
        StarknetRpcApiError::InternalServerError
    })?;
    let chain_id = config.chain_id;

    Ok(Felt(chain_id))
}
