use std::borrow::Cow;

use crate::{
    errors::{StarknetWsApiError, WsResult},
    versions::v0_8_0::StarknetWsRpcApiV0_8_0Server,
};

use super::BLOCK_PAST_LIMIT;

#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_8_0Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
        block_id: starknet_core::types::BlockId,
    ) -> WsResult {
        let Ok(sink) = pending.accept().await else {
            return WsResult::Err(StarknetWsApiError::internal(Cow::from("Failed to establish websocket connection")));
        };

        let mut block_n = match block_id {
            starknet_core::types::BlockId::Number(block_n) => {
                let Ok(Some(block_info)) = self.backend.get_block_info_from_block_latest() else {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                        "Failed to retrieve block info for block {block_n}",
                    ))));
                };

                if block_n < block_info.header.block_number.saturating_sub(BLOCK_PAST_LIMIT) {
                    return WsResult::Err(StarknetWsApiError::TooManyBlocksBack);
                }

                block_n
            }
            starknet_core::types::BlockId::Hash(block_hash) => {
                let Ok(Some(block_n)) = self.backend.block_hash_to_block_n(&block_hash) else {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                        "Failed to retrieve block info at hash {block_hash:#x}"
                    ))));
                };

                block_n
            }
            starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Latest) => {
                let Ok(Some(block_n)) = self.backend.get_latest_block_n() else {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                        "Failed to retrieve block info for latest block"
                    ))));
                };

                block_n
            }
            starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending) => {
                return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                    "`starknet_subscribeNewHeads` does not support pending blocks"
                ))))
            }
        };

        let mut rx = self.backend.subscribe_block_info();
        for n in block_n.. {
            if sink.is_closed() {
                return WsResult::Ok;
            }

            let block_info = match self.backend.get_block_info_from_block_n(n) {
                Ok(Some(block_info)) => block_info,
                Ok(None) => break,
                Err(e) => {
                    return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
                        "Failed to retrieve block info for block {n}: {e}"
                    ))))
                }
            };

            let res = send_block_header(&sink, block_info, block_n).await;
            if matches!(res, WsResult::Err(_)) {
                return res;
            }

            block_n += 1;
        }

        loop {
            let Ok(block_info) = rx.recv().await else {
                return WsResult::Err(StarknetWsApiError::internal(Cow::from("Failed to retrieve block info")));
            };

            if block_info.header.block_number == block_n {
                let res = send_block_header(&sink, block_info, block_n).await;
                if matches!(res, WsResult::Err(_)) {
                    return res;
                }

                break;
            }
        }

        loop {
            tokio::select! {
                block_info = rx.recv() => {
                    let Ok(block_info) = block_info else {
                        return WsResult::Err(StarknetWsApiError::internal(Cow::from("Failed to retrieve block info")));
                    };

                    let res = send_block_header(&sink, block_info, block_n).await;
                    if matches!(res, WsResult::Err(_)) {
                        return res;
                    }
                },
                _ = sink.closed() => {
                    return WsResult::Ok
                }
            }
        }
    }
}

async fn send_block_header<'a>(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    block_info: mp_block::MadaraBlockInfo,
    block_n: u64,
) -> WsResult<'a> {
    let header = starknet_api::block::BlockHeader::from(block_info);
    let Ok(msg) = jsonrpsee::SubscriptionMessage::from_json(&header) else {
        return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
            "Failed to create response message on block {block_n}"
        ))));
    };

    if let Err(e) = sink.send(msg).await {
        return WsResult::Err(StarknetWsApiError::internal(Cow::from(format!(
            "Failed to respond to websocket request: {e}"
        ))));
    }

    WsResult::Ok
}
