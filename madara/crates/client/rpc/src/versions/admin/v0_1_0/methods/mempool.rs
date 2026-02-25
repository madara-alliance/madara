use crate::{versions::admin::v0_1_0::MadaraMempoolRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};

#[async_trait]
impl MadaraMempoolRpcApiV0_1_0Server for Starknet {
    async fn set_mempool_intake(&self, enabled: bool) -> RpcResult<()> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .set_mempool_intake(enabled)
            .map_err(StarknetRpcApiError::from)?)
    }
}
