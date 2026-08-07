pub mod lib;
pub mod subscribe_events;
pub mod subscribe_new_heads;
pub mod subscribe_new_transactions;
pub mod subscribe_transaction_status;

const BLOCK_PAST_LIMIT: u64 = 1024;
const ADDRESS_FILTER_LIMIT: u64 = 128;
const REORG_NOTIFICATION_METHOD: &str = "starknet_subscriptionReorg";
const NEW_HEADS_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewHeads";
const EVENTS_NOTIFICATION_METHOD: &str = "starknet_subscriptionEvents";
const TRANSACTION_STATUS_NOTIFICATION_METHOD: &str = "starknet_subscriptionTransactionStatus";
const NEW_TRANSACTION_NOTIFICATION_METHOD: &str = "starknet_subscriptionNewTransaction";

#[derive(serde::Serialize)]
struct StarknetSubscriptionNotification<'a, T> {
    jsonrpc: &'static str,
    method: &'static str,
    params: StarknetSubscriptionParams<'a, T>,
}

#[derive(serde::Serialize)]
struct StarknetSubscriptionParams<'a, T> {
    subscription_id: String,
    result: &'a T,
}

fn subscription_id_string(subscription_id: jsonrpsee::types::SubscriptionId<'_>) -> String {
    match subscription_id {
        jsonrpsee::types::SubscriptionId::Num(id) => id.to_string(),
        jsonrpsee::types::SubscriptionId::Str(id) => id.into_owned(),
    }
}

pub async fn send_starknet_subscription<T: serde::Serialize>(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    method: &'static str,
    result: &T,
) -> Result<(), crate::errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    let json = starknet_subscription_json(sink.subscription_id(), method, result)?;
    let msg = jsonrpsee::SubscriptionMessage::from_complete_message(json);
    match sink.send(msg).await {
        Ok(()) => {
            crate::metrics::ws_metrics().record_notification_sent(method);
            Ok(())
        }
        Err(err) => {
            crate::metrics::ws_metrics().record_notification_send_failure(method);
            Err(err).or_internal_server_error("Failed to send websocket notification")
        }
    }
}

fn starknet_subscription_json<T: serde::Serialize>(
    subscription_id: jsonrpsee::types::SubscriptionId<'_>,
    method: &'static str,
    result: &T,
) -> Result<String, crate::errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    let notification = StarknetSubscriptionNotification {
        jsonrpc: "2.0",
        method,
        params: StarknetSubscriptionParams { subscription_id: subscription_id_string(subscription_id), result },
    };
    serde_json::to_string(&notification).or_internal_server_error("Failed to create websocket notification")
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
    send_starknet_subscription(sink, REORG_NOTIFICATION_METHOD, &reorg_data(reorg)).await?;
    crate::metrics::ws_metrics().record_reorg_notification_sent();
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonrpsee::types::SubscriptionId;
    use serde_json::json;
    use starknet_types_core::felt::Felt;

    #[test]
    fn starknet_subscription_json_matches_openrpc_envelope() {
        assert_eq!(NEW_HEADS_NOTIFICATION_METHOD, "starknet_subscriptionNewHeads");
        assert_eq!(EVENTS_NOTIFICATION_METHOD, "starknet_subscriptionEvents");
        assert_eq!(TRANSACTION_STATUS_NOTIFICATION_METHOD, "starknet_subscriptionTransactionStatus");
        assert_eq!(NEW_TRANSACTION_NOTIFICATION_METHOD, "starknet_subscriptionNewTransaction");
        assert_eq!(REORG_NOTIFICATION_METHOD, "starknet_subscriptionReorg");

        let result = json!({ "block_hash": "0x1" });
        let notification =
            starknet_subscription_json(SubscriptionId::Str("42".into()), NEW_HEADS_NOTIFICATION_METHOD, &result)
                .expect("serialize notification");
        let notification: serde_json::Value = serde_json::from_str(&notification).expect("parse notification");

        assert_eq!(notification["jsonrpc"], "2.0");
        assert_eq!(notification["method"], "starknet_subscriptionNewHeads");
        assert_eq!(notification["params"]["subscription_id"], "42");
        assert_eq!(notification["params"]["result"], result);
        assert!(notification["params"].get("subscription").is_none());
    }

    #[test]
    fn websocket_result_types_match_openrpc_shapes() {
        let event = mp_rpc::v0_10_2::EmittedEventWithFinality {
            emitted_event: mp_rpc::v0_10_2::EmittedEvent {
                event: mp_rpc::v0_10_2::Event {
                    from_address: Felt::from_hex_unchecked("0x1"),
                    event_content: mp_rpc::v0_10_2::EventContent { data: vec![], keys: vec![] },
                },
                block_hash: None,
                block_number: None,
                transaction_hash: Felt::from_hex_unchecked("0x2"),
                transaction_index: 0,
                event_index: 0,
            },
            finality_status: mp_rpc::v0_10_2::TxnFinalityStatus::PreConfirmed,
        };
        let event = serde_json::to_value(event).expect("serialize event");
        assert_eq!(event["finality_status"], "PRE_CONFIRMED");
        assert!(event.get("PRE_CONFIRMED").is_none());

        let status = mp_rpc::v0_10_2::NewTxnStatus {
            transaction_hash: Felt::from_hex_unchecked("0x3"),
            status: mp_rpc::v0_10_2::WsTxnStatusResult {
                execution_status: None,
                finality_status: mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2,
                failure_reason: None,
            },
        };
        let status = serde_json::to_value(status).expect("serialize transaction status");
        assert_eq!(status["transaction_hash"], "0x3");
        assert_eq!(status["status"]["finality_status"], "ACCEPTED_ON_L2");
        assert!(status["status"].get("execution_status").is_none());
        assert!(status["status"].get("failure_reason").is_none());
    }
}
