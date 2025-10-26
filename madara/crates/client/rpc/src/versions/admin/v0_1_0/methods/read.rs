use crate::{versions::admin::v0_1_0::MadaraReadRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use blockifier::bouncer::BouncerWeights;
use jsonrpsee::core::{async_trait, RpcResult};

#[async_trait]
impl MadaraReadRpcApiV0_1_0Server for Starknet {
    async fn get_block_builtin_weights(&self, block_number: u64) -> RpcResult<BouncerWeights> {
        let block_view =
            self.backend.block_view_on_confirmed(block_number).ok_or(StarknetRpcApiError::BlockNotFound)?;
        let bouncer_weights = block_view.get_bouncer_weights().map_err(StarknetRpcApiError::from)?;
        Ok(bouncer_weights)
    }
}
