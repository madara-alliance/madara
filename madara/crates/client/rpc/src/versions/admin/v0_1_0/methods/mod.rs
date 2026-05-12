use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::RpcResult;

pub mod read;
pub mod services;
pub mod status;
pub mod write;
#[cfg(feature = "replay")]
mod write_replay;

fn ensure_rpc_unsafe_enabled(starknet: &Starknet) -> RpcResult<()> {
    if !starknet.rpc_unsafe_enabled {
        return Err(StarknetRpcApiError::ErrUnexpectedError {
            error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
        }
        .into());
    }

    Ok(())
}
