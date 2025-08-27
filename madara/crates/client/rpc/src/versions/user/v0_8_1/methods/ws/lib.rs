use mp_block::BlockId;
use starknet_types_core::felt::Felt;

use crate::{versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server, StarknetRpcApiError};

use super::starknet_unsubscribe::*;
// use super::subscribe_events::*;
// use super::subscribe_new_heads::*;
// use super::subscribe_pending_transactions::*;
// use super::subscribe_transaction_status::*;

#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_8_1Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block: BlockId,
    ) -> jsonrpsee::core::SubscriptionResult {
        // Ok(subscribe_new_heads(self, subscription_sink, block).await?)
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }

    async fn subscribe_events(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        from_address: Option<Felt>,
        keys: Option<Vec<Vec<Felt>>>,
        block: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult {
        // Ok(subscribe_events(self, subscription_sink, from_address, keys, block).await?)
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }

    async fn subscribe_transaction_status(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::SubscriptionResult {
        // Ok(subscribe_transaction_status(self, subscription_sink, transaction_hash).await?)
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }

    async fn subscribe_pending_transactions(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_details: bool,
        sender_address: Vec<starknet_types_core::felt::Felt>,
    ) -> jsonrpsee::core::SubscriptionResult {
        // Ok(subscribe_pending_transactions(self, subscription_sink, transaction_details, sender_address).await?)
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }

    async fn starknet_unsubscribe(&self, subscription_id: u64) -> jsonrpsee::core::RpcResult<bool> {
        Ok(starknet_unsubscribe(self, subscription_id).await?)
    }
}
