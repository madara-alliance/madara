use crate::{versions::admin::v0_1_0::MadaraExecutionBoxRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_block_production::fallback::manager::EnableOutcome;
use mc_block_production::fallback::types::ExecutionboxStatus;

#[async_trait]
impl MadaraExecutionBoxRpcApiV0_1_0Server for Starknet {
    async fn executionbox_disable(&self) -> RpcResult<()> {
        self.block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .executionbox_disable()
            .await
            .map_err(|e| {
                StarknetRpcApiError::ErrUnexpectedError {
                    error: format!("Block production channel closed: {e}").into(),
                }
                .into()
            })
    }

    async fn executionbox_enable(&self) -> RpcResult<String> {
        let handle = self.block_prod_handle.as_ref().ok_or(StarknetRpcApiError::UnimplementedMethod)?;

        match handle.executionbox_enable().await {
            Err(_channel_err) => Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "Block production channel closed".to_string().into(),
            }
            .into()),
            Ok(Err(mc_block_production::fallback::manager::EnableError::ReplayInProgress)) => {
                // V1: this fires only during startup recovery (no runtime replay yet).
                Err(StarknetRpcApiError::ErrUnexpectedError {
                    error: "replay_in_progress: startup recovery still active or replay backlog not drained"
                        .to_string()
                        .into(),
                }
                .into())
            }
            Ok(Ok(EnableOutcome::EnabledNow)) => Ok("enabled_now".to_string()),
            Ok(Ok(EnableOutcome::AlreadyMixed)) => Ok("already_mixed".to_string()),
        }
    }

    async fn executionbox_status(&self) -> RpcResult<ExecutionboxStatus> {
        self.block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .executionbox_status()
            .await
            .map_err(|e| {
                StarknetRpcApiError::ErrUnexpectedError {
                    error: format!("Block production channel closed: {e}").into(),
                }
                .into()
            })
    }
}
