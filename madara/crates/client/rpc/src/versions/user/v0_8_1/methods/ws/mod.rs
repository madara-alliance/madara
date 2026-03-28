pub mod lib;
pub mod starknet_unsubscribe;
// FIXME(subscriptions): Re-add subscriptions.
pub mod subscribe_events;
// FIXME(subscriptions): Re-add subscriptions.
pub mod subscribe_new_heads;
// FIXME(subscriptions): Re-add subscriptions.
pub mod subscribe_pending_transactions;
// FIXME(subscriptions): Re-add subscriptions.
pub mod subscribe_transaction_status;

// FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
#[allow(unused)]
const BLOCK_PAST_LIMIT: u64 = 1024;
// FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
#[allow(unused)]
const ADDRESS_FILTER_LIMIT: u64 = 128;
const REORG_NOTIFICATION_METHOD: &str = "starknet_subscriptionReorg";

#[derive(PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionItem<T> {
    subscription_id: u64,
    result: T,
}

impl<T> SubscriptionItem<T> {
    pub fn new(subscription_id: jsonrpsee::types::SubscriptionId, result: T) -> Self {
        let subscription_id = match subscription_id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(_) => {
                unreachable!("Jsonrpsee middleware has been configured to use u64 subscription ids")
            }
        };

        Self { subscription_id, result }
    }
}

pub fn reorg_data(reorg: &mc_db::ReorgNotification) -> mp_rpc::v0_8_1::ReorgData {
    mp_rpc::v0_8_1::ReorgData {
        starting_block_hash: reorg.first_reverted_block_hash,
        starting_block_number: reorg.first_reverted_block_n,
        ending_block_hash: reorg.previous_head.latest_confirmed_block_hash,
        ending_block_number: reorg.previous_head.latest_confirmed_block_n,
    }
}

pub async fn send_reorg_notification(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    reorg: &mc_db::ReorgNotification,
) -> Result<(), crate::errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    let msg =
        jsonrpsee::SubscriptionMessage::new(REORG_NOTIFICATION_METHOD, sink.subscription_id(), &reorg_data(reorg))
            .or_else_internal_server_error(|| "Failed to create reorg websocket notification")?;

    sink.send(msg).await.or_internal_server_error("Failed to send reorg websocket notification")
}
