use std::sync::Arc;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sp_api::ProvideRuntimeApi;
use sp_runtime::traits::Block as BlockT;
pub use mc_rpc_core::BlockNumberServer;
use sp_runtime::DispatchError;
use crate::errors::StarknetRpcApiError;

pub fn convert_error<C, B, T>(
    client: Arc<C>,
    best_block_hash: <B as BlockT>::Hash,
    call_result: Result<T, DispatchError>,
) -> Result<T, StarknetRpcApiError>
where
    B: BlockT,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B> + ConvertTransactionRuntimeApi<B>,
{
    match call_result {
        Ok(val) => Ok(val),
        Err(e) => match client.runtime_api().convert_error(best_block_hash, e) {
            Ok(starknet_error) => Err(starknet_error.into()),
            Err(_) => Err(StarknetRpcApiError::InternalServerError),
        },
    }
}