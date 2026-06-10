pub mod lib;
pub mod starknet_unsubscribe;
pub mod subscribe_events;
pub mod subscribe_new_heads;
pub mod subscribe_new_transaction_receipts;
pub mod subscribe_new_transactions;
pub mod subscribe_transaction_status;

#[allow(unused)]
const BLOCK_PAST_LIMIT: u64 = 1024;
#[allow(unused)]
const ADDRESS_FILTER_LIMIT: u64 = 128;
const REORG_NOTIFICATION_METHOD: &str = "starknet_subscriptionReorg";
const NEW_HEADS_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewHeads";
const EVENTS_NOTIFICATION_METHOD: &str = "starknet_subscriptionEvents";
const TRANSACTION_STATUS_NOTIFICATION_METHOD: &str = "starknet_subscriptionTransactionStatus";
const NEW_TRANSACTION_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewTransaction";
const NEW_TRANSACTION_RECEIPTS_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewTransactionReceipts";

/// Builds a spec-shaped subscription notification frame.
///
/// The spec requires notifications to use the dedicated `starknet_subscriptionX` method names,
/// not the subscribe method name jsonrpsee would default to. [`jsonrpsee::SubscriptionMessage::new`]
/// produces a complete frame with the given method, the subscription id, and the payload as
/// `params.result`.
pub(crate) fn notification_message<T: serde::Serialize>(
    method: &str,
    sink: &jsonrpsee::core::server::SubscriptionSink,
    payload: &T,
) -> Result<jsonrpsee::SubscriptionMessage, serde_json::Error> {
    jsonrpsee::SubscriptionMessage::new(method, sink.subscription_id(), payload)
}

pub fn reorg_data(reorg: &mc_db::ReorgNotification) -> mp_rpc::v0_10_2::ReorgData {
    mp_rpc::v0_10_2::ReorgData {
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

pub fn missed_reorg_notifications_error() -> crate::errors::StarknetWsApiError {
    crate::errors::StarknetWsApiError::internal_server_error(
        "Missed reorg notifications; websocket subscription can no longer guarantee canonical state",
    )
}

pub fn missed_received_transaction_notifications_error() -> crate::errors::StarknetWsApiError {
    crate::errors::StarknetWsApiError::internal_server_error(
        "Missed new-transaction notifications; websocket subscription can no longer guarantee received transaction updates",
    )
}
