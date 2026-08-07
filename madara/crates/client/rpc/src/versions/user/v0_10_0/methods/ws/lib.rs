use mp_rpc::v0_10_0::{BlockId, BlockTag, FinalityStatus, TxnStatusWithoutL1};
use starknet_types_core::felt::Felt;

use crate::versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server;
use crate::versions::user::v0_10_2::methods::ws::{
    subscribe_events::subscribe_events as subscribe_events_v0_10_2,
    subscribe_new_heads::subscribe_new_heads as subscribe_new_heads_v0_10_2,
    subscribe_transaction_status::subscribe_transaction_status as subscribe_transaction_status_v0_10_2,
};

use super::starknet_unsubscribe::*;
use super::subscribe_new_transaction_receipts::subscribe_new_transaction_receipts_with_reorg;
use super::subscribe_new_transactions::subscribe_new_transactions_with_reorg;

#[jsonrpsee::core::async_trait]
#[allow(unused)]
impl StarknetWsRpcApiV0_10_0Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block_id: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_new_heads_v0_10_2(self, subscription_sink, block_id.unwrap_or(BlockId::Tag(BlockTag::Latest)))
            .await?)
    }

    async fn subscribe_events(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        from_address: Option<Felt>,
        keys: Option<Vec<Vec<Felt>>>,
        block: Option<BlockId>,
        finality_status: Option<FinalityStatus>,
    ) -> jsonrpsee::core::SubscriptionResult {
        let from_address = from_address.map(mp_rpc::v0_10_2::AddressFilter::Single);
        Ok(subscribe_events_v0_10_2(self, subscription_sink, from_address, keys, block, finality_status).await?)
    }

    async fn subscribe_transaction_status(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_transaction_status_v0_10_2(self, subscription_sink, transaction_hash).await?)
    }

    async fn subscribe_new_transactions(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        finality_status: Option<Vec<TxnStatusWithoutL1>>,
        sender_address: Option<Vec<starknet_types_core::felt::Felt>>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_new_transactions_with_reorg(self, subscription_sink, finality_status, sender_address).await?)
    }

    async fn subscribe_new_transaction_receipts(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        finality_status: Option<Vec<FinalityStatus>>,
        sender_address: Option<Vec<starknet_types_core::felt::Felt>>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_new_transaction_receipts_with_reorg(self, subscription_sink, finality_status, sender_address)
            .await?)
    }

    async fn starknet_unsubscribe(&self, subscription_id: String) -> jsonrpsee::core::RpcResult<bool> {
        Ok(starknet_unsubscribe(self, subscription_id).await?)
    }
}
