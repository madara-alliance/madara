use crate::errors::{ErrorExtWs, OptionExtWs, StarknetWsApiError};
use anyhow::Context;
use mc_db::{subscription::SubscribeNewBlocksTag, EventFilter};
use mp_block::EventWithInfo;
use mp_rpc::v0_9_0::{BlockId, BlockTag, EmittedEvent, FinalityStatus, TxnFinalityStatus};
use starknet_types_core::felt::Felt;

use super::BLOCK_PAST_LIMIT;

pub async fn subscribe_events(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    from_address: Option<Felt>,
    keys: Option<Vec<Vec<Felt>>>,
    block_id: Option<BlockId>,
    finality_status: Option<FinalityStatus>,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;
    let requested_finality = finality_status.unwrap_or_default();

    let mut next_block_n = starknet.backend.latest_confirmed_block_n().map_or(0, |block_n| block_n.saturating_add(1));

    if let Some(block_id) = block_id {
        if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
            return Err(StarknetWsApiError::Pending);
        }

        let view = match starknet.resolve_view_on(block_id) {
            Ok(view) => view,
            Err(crate::StarknetRpcApiError::BlockNotFound) => return Err(StarknetWsApiError::BlockNotFound),
            Err(crate::StarknetRpcApiError::NoBlocks) => return Err(StarknetWsApiError::NoBlocks),
            Err(err) => return Err(StarknetWsApiError::internal_server_error(err.to_string())),
        };
        let latest_block = starknet.backend.view_on_latest().latest_block_n().ok_or(StarknetWsApiError::NoBlocks)?;
        let block_n = view.latest_block_n().ok_or(StarknetWsApiError::NoBlocks)?;

        if block_n < latest_block.saturating_sub(BLOCK_PAST_LIMIT) {
            return Err(StarknetWsApiError::TooManyBlocksBack);
        }

        for block_number in block_n..=latest_block {
            send_block_events(starknet, &sink, &from_address, &keys, block_number, &requested_finality).await?;
        }

        next_block_n = latest_block.saturating_add(1);
    }

    let mut heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
    heads.set_start_from(next_block_n);
    let mut reorgs = starknet.backend.subscribe_reorgs();

    loop {
        let block_view = tokio::select! {
            block_view = heads.next_block_view() => block_view,
            reorg = reorgs.recv() => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                        heads.set_start_from(reorg.first_reverted_block_n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            },
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
        };

        let block_number = block_view.block_number();
        send_block_events(starknet, &sink, &from_address, &keys, block_number, &requested_finality).await?;
    }
}

async fn send_block_events(
    starknet: &crate::Starknet,
    sink: &jsonrpsee::core::server::SubscriptionSink,
    from_address: &Option<Felt>,
    keys: &Option<Vec<Vec<Felt>>>,
    block_number: u64,
    requested_finality: &FinalityStatus,
) -> Result<(), StarknetWsApiError> {
    let view = starknet.backend.view_on_latest();

    let events = view
        .get_events(EventFilter {
            start_block: block_number,
            start_event_index: 0,
            end_block: block_number,
            from_address: from_address.clone(),
            keys_pattern: keys.clone(),
            max_events: usize::MAX,
        })
        .context("Error getting filtered events")
        .or_internal_server_error("Failed to retrieve filtered events")?;

    for event in events {
        let finality_status = event_finality_status(starknet, &event)?;
        if !subscription_allows_finality(requested_finality, finality_status) {
            continue;
        }

        send_event(event, finality_status, sink).await?;
    }

    Ok(())
}

fn subscription_allows_finality(requested_finality: &FinalityStatus, event_finality: TxnFinalityStatus) -> bool {
    match requested_finality {
        FinalityStatus::PreConfirmed => matches!(event_finality, TxnFinalityStatus::PreConfirmed),
        FinalityStatus::AcceptedOnL2 => !matches!(event_finality, TxnFinalityStatus::PreConfirmed),
    }
}

fn event_finality_status(
    starknet: &crate::Starknet,
    event: &EventWithInfo,
) -> Result<TxnFinalityStatus, StarknetWsApiError> {
    if event.in_preconfirmed {
        return Ok(TxnFinalityStatus::PreConfirmed);
    }

    let block_view =
        starknet.backend.block_view_on_confirmed(event.block_number).ok_or_else_internal_server_error(|| {
            format!("Failed to retrieve confirmed block for block {}", event.block_number)
        })?;

    Ok(if block_view.is_on_l1() { TxnFinalityStatus::L1 } else { TxnFinalityStatus::L2 })
}

async fn send_event(
    event: EventWithInfo,
    finality_status: TxnFinalityStatus,
    sink: &jsonrpsee::core::server::SubscriptionSink,
) -> Result<(), StarknetWsApiError> {
    let emitted_event = EmittedEvent::from(event);
    let item = super::SubscriptionItem::new(
        sink.subscription_id(),
        mp_rpc::v0_9_0::EmittedEventWithFinality { emmitted_event: emitted_event, finality_status },
    );
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_internal_server_error("Failed to create response message")?;
    sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{
        test_utils::rpc_test_setup,
        versions::user::v0_9_0::{StarknetWsRpcApiV0_9_0Client, StarknetWsRpcApiV0_9_0Server},
        Starknet,
    };
    use jsonrpsee::{
        core::{client::SubscriptionClientT, params::ObjectParams},
        ws_client::WsClientBuilder,
    };
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
    use mp_chain_config::StarknetVersion;
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::v0_9_0::{FinalityStatus, TxnFinalityStatus};
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction as MpTransaction};
    use serde_json::Value;
    use std::time::Duration;

    const SERVER_ADDR: &str = "127.0.0.1:0";

    fn transaction_with_event(
        sender_address: Felt,
        transaction_hash: Felt,
        event_from_address: Felt,
    ) -> TransactionWithReceipt {
        TransactionWithReceipt {
            transaction: MpTransaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                contract_address: sender_address,
                ..Default::default()
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                events: vec![mp_receipt::Event { from_address: event_from_address, keys: vec![], data: vec![] }],
                actual_fee: FeePayment { amount: Felt::from(0u8), unit: PriceUnit::Wei },
                messages_sent: vec![],
                execution_resources: ExecutionResources::default(),
                execution_result: ExecutionResult::Succeeded,
            }),
        }
    }

    fn add_confirmed_event_block(
        backend: &std::sync::Arc<mc_db::MadaraBackend>,
        block_number: u64,
        sender_address: Felt,
        event_from_address: Felt,
        transaction_hash: Felt,
    ) {
        let tx = transaction_with_event(sender_address, transaction_hash, event_from_address);
        let events = tx
            .receipt
            .events()
            .iter()
            .cloned()
            .map(|event| mp_receipt::EventWithTransactionHash { transaction_hash, event })
            .collect::<Vec<_>>();

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number, ..Default::default() },
                    state_diff: mp_state_update::StateDiff::default(),
                    transactions: vec![tx],
                    events,
                },
                &[],
                false,
            )
            .expect("Storing block");
    }

    async fn start_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder().build(SERVER_ADDR).await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_events(
        client: &jsonrpsee::ws_client::WsClient,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_9_0_subscribeEvents",
            ObjectParams::new(),
            "starknet_V0_9_0_unsubscribe",
        )
        .await
        .expect("starknet_V0_9_0_subscribeEvents")
    }

    #[tokio::test]
    async fn subscribe_events_default_finality_emits_confirmed_events() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = client.subscribe_events(None, None, None, None).await.expect("Failed subscription");

        let tx_hash = Felt::from_hex_unchecked("0x4242");
        let event_from_address = Felt::from_hex_unchecked("0x1234");
        add_confirmed_event_block(&backend, 0, event_from_address, event_from_address, tx_hash);

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve event");

        assert_eq!(item.result.finality_status, TxnFinalityStatus::L2);
        assert_eq!(item.result.emmitted_event.event.from_address, event_from_address);
    }

    #[tokio::test]
    async fn subscribe_events_preconfirmed_finality_emits_preconfirmed_events() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = client
            .subscribe_events(None, None, None, Some(FinalityStatus::PreConfirmed))
            .await
            .expect("Failed subscription");

        let event_from_address = Felt::from_hex_unchecked("0x2345");
        let transaction_hash = Felt::from_hex_unchecked("0x4243");
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    block_number: 0,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: transaction_with_event(event_from_address, transaction_hash, event_from_address),
                    state_diff: Default::default(),
                    declared_class: None,
                    arrived_at: Default::default(),
                    paid_fee_on_l1: None,
                }],
                vec![],
            ))
            .expect("Failed to store preconfirmed block");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve event");

        assert_eq!(item.result.finality_status, TxnFinalityStatus::PreConfirmed);
        assert!(item.result.emmitted_event.block_number.is_none());
        assert_eq!(item.result.emmitted_event.event.from_address, event_from_address);
    }

    #[tokio::test]
    async fn subscribe_events_reorg_then_resume() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let block_0_tx_hash = Felt::from_hex_unchecked("0x7000");
        let block_1_tx_hash = Felt::from_hex_unchecked("0x7001");
        let replacement_tx_hash = Felt::from_hex_unchecked("0x7002");
        let event_from_address = Felt::from_hex_unchecked("0x1234");
        add_confirmed_event_block(&backend, 0, event_from_address, event_from_address, block_0_tx_hash);
        add_confirmed_event_block(&backend, 1, event_from_address, event_from_address, block_1_tx_hash);

        let block_0_hash = backend
            .block_view_on_confirmed(0)
            .expect("Retrieving block 0 view")
            .get_block_info()
            .expect("Retrieving block 0 info")
            .block_hash;
        let block_1_hash = backend
            .block_view_on_confirmed(1)
            .expect("Retrieving block 1 view")
            .get_block_info()
            .expect("Retrieving block 1 info")
            .block_hash;

        let mut sub = raw_subscribe_events(&client).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_9_0::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_1_hash,
                ending_block_number: 1,
            })
            .expect("Failed to serialize expected reorg notification")
        );

        add_confirmed_event_block(&backend, 1, event_from_address, event_from_address, replacement_tx_hash);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement event");
        let item: super::super::SubscriptionItem<mp_rpc::v0_9_0::EmittedEventWithFinality> =
            serde_json::from_value(next).expect("Failed to deserialize event item");
        let event = item.result;

        assert_eq!(event.finality_status, TxnFinalityStatus::L2);
        assert_eq!(event.emmitted_event.event.from_address, event_from_address);
    }
}
