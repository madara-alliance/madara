use mp_block::BlockId;
use starknet_types_core::felt::Felt;

use crate::versions::user::v0_8_0::StarknetWsRpcApiV0_8_0Server;

use super::subscribe_events::*;
use super::subscribe_new_heads::*;
use super::subscribe_transaction_status::*;

#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_8_0Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block: BlockId,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_new_heads(self, subscription_sink, block).await?)
    }

    async fn subscribe_events(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        from_address: Option<Felt>,
        keys: Option<Vec<Vec<Felt>>>,
        block: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_events(self, subscription_sink, from_address, keys, block).await?)
    }

    async fn subscribe_transaction_status(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_transaction_status(self, subscription_sink, transaction_hash).await?)
    }
}
