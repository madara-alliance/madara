use mp_rpc::v0_10_0::BlockId;
use starknet_types_core::felt::Felt;

use crate::versions::user::v0_9_0::StarknetWsRpcApiV0_9_0Server as V0_9_0Impl;
use crate::{versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server, StarknetRpcApiError};

use super::starknet_unsubscribe::*;

// All subscription types are compatible (BlockId is a type alias to v0.9.0::BlockId),
// and since subscriptions are disabled across all versions (they return UnimplementedMethod),
// we can delegate all subscription methods to v0.9.0

#[jsonrpsee::core::async_trait]
// FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
#[allow(unused)]
impl StarknetWsRpcApiV0_10_0Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block: BlockId,
    ) -> jsonrpsee::core::SubscriptionResult {
        // BlockId is the same type as v0.9.0 (via type alias), so we can delegate directly
        V0_9_0Impl::subscribe_new_heads(self, subscription_sink, block).await
    }

    async fn subscribe_events(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        from_address: Option<Felt>,
        keys: Option<Vec<Vec<Felt>>>,
        block: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_9_0Impl::subscribe_events(self, subscription_sink, from_address, keys, block).await
    }

    async fn subscribe_transaction_status(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_9_0Impl::subscribe_transaction_status(self, subscription_sink, transaction_hash).await
    }

    async fn subscribe_pending_transactions(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_details: bool,
        sender_address: Vec<starknet_types_core::felt::Felt>,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_9_0Impl::subscribe_pending_transactions(self, subscription_sink, transaction_details, sender_address).await
    }

    async fn starknet_unsubscribe(&self, subscription_id: u64) -> jsonrpsee::core::RpcResult<bool> {
        Ok(starknet_unsubscribe(self, subscription_id).await?)
    }
}
