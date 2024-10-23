use std::borrow::Cow;

use crate::{
    errors::{StarknetWsApiError, WsResult},
    versions::v0_8_0::StarknetWsRpcApiV0_8_0Server,
};

use super::BLOCK_PAST_LIMIT;

#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_8_0Server for crate::Starknet {
    async fn foo(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
        block_id: starknet_core::types::BlockId,
    ) -> jsonrpsee::core::SubscriptionResult {
        let sink = pending.accept().await?;

        let msg = jsonrpsee::SubscriptionMessage::from(format!("{block_id:?}"));
        sink.send(msg).await?;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        #[derive(serde::Serialize)]
        struct StarknetSubscriptionError<'a> {
            code: u32,
            message: &'a str,
        }

        let err = serde_json::to_string(&StarknetSubscriptionError {
            code: 68,
            message: "Cannot go back more than 1024 blocks",
        })
        .unwrap();

        return Err(err.into());
    }

    async fn subscribe_new_heads(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
        block_id: starknet_core::types::BlockId,
    ) -> WsResult {
        let Ok(sink) = pending.accept().await else {
            return WsResult::Err(StarknetWsApiError::internal(Cow::from("Failed to establish websocket connection")));
        };

        match block_id {
            starknet_core::types::BlockId::Number(block_n) => {
                let block_info = self.backend.get_block_info(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest));

                let Ok(Some(block_info)) = block_info else {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(
                        "Failed to retrieve latest block info from database",
                    )));
                };

                let Some(block_info) = block_info.as_nonpending_owned() else {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(
                        "Retrieve pending block info for non-pending block",
                    )));
                };

                let target = block_info.header.block_number.saturating_sub(BLOCK_PAST_LIMIT);

                if block_n < target {
                    return WsResult::Err(StarknetWsApiError::TooManyBlocksBack);
                }

                let header = starknet_api::block::BlockHeader::from(block_info);
                let msg = jsonrpsee::SubscriptionMessage::from_json(&header).unwrap();
                if let Err(e) = sink.send(msg).await {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                        "Failed to respond to websocket request: {e}"
                    ))));
                }
            }
            _ => todo!(),
        }

        return WsResult::Ok;
    }
}
