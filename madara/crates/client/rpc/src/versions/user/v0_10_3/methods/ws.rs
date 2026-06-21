use starknet_types_core::felt::Felt;

use crate::versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server as V0_10_2Impl;
use crate::versions::user::v0_10_3::StarknetWsRpcApiV0_10_3Server;

use mp_rpc::v0_10_0::BlockId;

// v0.10.3 has no semantic changes to the websocket API: delegate to v0.10.2.
#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_10_3Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block_id: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_10_2Impl::subscribe_new_heads(self, subscription_sink, block_id).await
    }

    async fn subscribe_events(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        from_address: Option<mp_rpc::v0_10_3::AddressFilter>,
        keys: Option<Vec<Vec<Felt>>>,
        block_id: Option<BlockId>,
        finality_status: Option<mp_rpc::v0_10_3::FinalityStatus>,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_10_2Impl::subscribe_events(self, subscription_sink, from_address, keys, block_id, finality_status).await
    }

    async fn subscribe_transaction_status(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_10_2Impl::subscribe_transaction_status(self, subscription_sink, transaction_hash).await
    }

    async fn subscribe_new_transactions(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        finality_status: Option<Vec<mp_rpc::v0_10_3::TxnStatusWithoutL1>>,
        sender_address: Option<Vec<Felt>>,
        tags: Option<Vec<mp_rpc::v0_10_3::SubscriptionTag>>,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_10_2Impl::subscribe_new_transactions(self, subscription_sink, finality_status, sender_address, tags).await
    }

    async fn subscribe_new_transaction_receipts(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        finality_status: Option<Vec<mp_rpc::v0_10_3::FinalityStatus>>,
        sender_address: Option<Vec<Felt>>,
    ) -> jsonrpsee::core::SubscriptionResult {
        V0_10_2Impl::subscribe_new_transaction_receipts(self, subscription_sink, finality_status, sender_address).await
    }

    async fn starknet_unsubscribe(&self, subscription_id: String) -> jsonrpsee::core::RpcResult<bool> {
        V0_10_2Impl::starknet_unsubscribe(self, subscription_id).await
    }
}
