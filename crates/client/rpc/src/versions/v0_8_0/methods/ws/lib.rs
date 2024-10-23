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
            starknet_core::types::BlockId::Number(mut block_n) => {
                let Ok(Some(block_info)) = self.backend.get_block_info_from_block_latest() else {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(
                        "Failed to retrieve latest block info from database",
                    )));
                };

                if block_n < block_info.header.block_number.saturating_sub(BLOCK_PAST_LIMIT) {
                    return WsResult::Err(StarknetWsApiError::TooManyBlocksBack);
                }

                loop {
                    for n in block_n.. {
                        let block_info = match self.backend.get_block_info_from_block_n(n) {
                            Ok(Some(block_info)) => block_info,
                            Ok(None) => break,
                            Err(e) => {
                                return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                                    "Failed to retrieve block info for block {n}: {e}"
                                ))))
                            }
                        };

                        log::info!("Sending block {n}");

                        let header = starknet_api::block::BlockHeader::from(block_info);
                        let msg = jsonrpsee::SubscriptionMessage::from_json(&header).unwrap();
                        if let Err(e) = sink.send(msg).await {
                            return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                                "Failed to respond to websocket request: {e}"
                            ))));
                        }
                    }

                    log::info!("Waiting for new blocks");

                    self.backend.wait_for_block_info().await;

                    log::info!("New blocks received");

                    block_n = block_info.header.block_number + 1;
                }
            }
            _ => todo!(),
        }
    }
}
