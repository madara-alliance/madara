use crate::{versions::admin::v0_1_0::MadaraMempoolRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};

#[async_trait]
impl MadaraMempoolRpcApiV0_1_0Server for Starknet {
    async fn set_mempool_intake(&self, enabled: bool) -> RpcResult<()> {
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .set_mempool_intake(enabled)
            .map_err(StarknetRpcApiError::from)?)
    }
}
