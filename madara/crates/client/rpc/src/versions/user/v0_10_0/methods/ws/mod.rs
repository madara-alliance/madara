pub mod lib;
pub mod starknet_unsubscribe;
pub mod subscribe_new_transaction_receipts;
pub mod subscribe_new_transactions;

const ADDRESS_FILTER_LIMIT: u64 = 128;
const REORG_NOTIFICATION_METHOD: &str = "starknet_subscriptionReorg";
const NEW_TRANSACTION_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewTransaction";
const NEW_TRANSACTION_RECEIPTS_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewTransactionReceipts";

pub fn reorg_data(reorg: &mc_db::ReorgNotification) -> mp_rpc::v0_10_0::ReorgData {
    mp_rpc::v0_10_0::ReorgData {
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
    crate::versions::user::v0_10_2::methods::ws::send_starknet_subscription(
        sink,
        REORG_NOTIFICATION_METHOD,
        &reorg_data(reorg),
    )
    .await
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
