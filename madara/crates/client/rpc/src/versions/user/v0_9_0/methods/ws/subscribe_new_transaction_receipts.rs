use crate::errors::ErrorExtWs;
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_9_0::{FinalityStatus, TxnFinalityStatus, TxnReceiptWithBlockInfo};
use mp_transactions::Transaction;
use starknet_types_core::felt::Felt;

use std::collections::HashSet;

pub async fn subscribe_new_transaction_receipts(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<FinalityStatus>>,
    sender_address: Option<Vec<Felt>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    subscribe_new_transaction_receipts_inner(starknet, subscription_sink, finality_status, sender_address, false).await
}

pub async fn subscribe_new_transaction_receipts_with_reorg(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<FinalityStatus>>,
    sender_address: Option<Vec<Felt>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    subscribe_new_transaction_receipts_inner(starknet, subscription_sink, finality_status, sender_address, true).await
}

async fn subscribe_new_transaction_receipts_inner(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<FinalityStatus>>,
    sender_address: Option<Vec<Felt>>,
    emit_reorg_notifications: bool,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let allowed_finality_status =
        finality_status.unwrap_or_else(|| vec![FinalityStatus::AcceptedOnL2]).into_iter().collect::<HashSet<_>>();
    let sender_address = sender_address.map(|addresses| addresses.into_iter().collect::<HashSet<_>>());

    let mut block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
    let mut reorgs = emit_reorg_notifications.then(|| starknet.backend.subscribe_reorgs());

    loop {
        let block_view = tokio::select! {
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
            reorg = async {
                match &mut reorgs {
                    Some(reorgs) => reorgs.recv().await,
                    None => std::future::pending::<Result<mc_db::ReorgNotification, tokio::sync::broadcast::error::RecvError>>().await,
                }
            } => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                        block_stream.set_start_from(reorg.first_reverted_block_n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            },
            block_view = block_stream.next_block_view() => block_view,
        };

        send_block_receipts(&sink, &allowed_finality_status, sender_address.as_ref(), block_view).await?;
    }
}

async fn send_block_receipts(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    allowed_finality_status: &HashSet<FinalityStatus>,
    sender_address: Option<&HashSet<Felt>>,
    block_view: mc_db::MadaraBlockView,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let (finality_status, block_hash) = match block_view.as_confirmed() {
        Some(confirmed) if confirmed.is_on_l1() => return Ok(()),
        Some(confirmed) => (
            FinalityStatus::AcceptedOnL2,
            Some(
                confirmed
                    .get_block_info()
                    .or_internal_server_error(
                        "SubscribeNewTransactionReceipts failed to retrieve confirmed block info",
                    )?
                    .block_hash,
            ),
        ),
        None => (FinalityStatus::PreConfirmed, None),
    };

    if !allowed_finality_status.contains(&finality_status) {
        return Ok(());
    }

    let block_number = block_view.block_number();
    for tx in block_view
        .get_executed_transactions(..)
        .or_internal_server_error("SubscribeNewTransactionReceipts failed to retrieve block transactions")?
    {
        if !transaction_matches_sender(&tx.transaction, sender_address) {
            continue;
        }

        let tx_hash = *tx.receipt.transaction_hash();
        let transaction_receipt = tx.receipt.to_rpc_v0_9(match finality_status {
            FinalityStatus::PreConfirmed => TxnFinalityStatus::PreConfirmed,
            FinalityStatus::AcceptedOnL2 => TxnFinalityStatus::L2,
        });
        let item = super::SubscriptionItem::new(
            sink.subscription_id(),
            TxnReceiptWithBlockInfo { transaction_receipt, block_hash, block_number },
        );
        let msg = jsonrpsee::SubscriptionMessage::from_json(&item).or_else_internal_server_error(|| {
            format!("SubscribeNewTransactionReceipts failed to create response for tx hash {tx_hash:#x}")
        })?;

        sink.send(msg)
            .await
            .or_internal_server_error("SubscribeNewTransactionReceipts failed to respond to websocket request")?;
    }

    Ok(())
}

fn transaction_matches_sender(transaction: &Transaction, sender_address: Option<&HashSet<Felt>>) -> bool {
    let Some(sender_address) = sender_address else {
        return true;
    };

    match transaction {
        Transaction::Invoke(inner) => sender_address.contains(inner.sender_address()),
        Transaction::L1Handler(inner) => sender_address.contains(&inner.contract_address),
        Transaction::Declare(inner) => sender_address.contains(inner.sender_address()),
        Transaction::Deploy(inner) => sender_address.contains(&inner.calculate_contract_address()),
        Transaction::DeployAccount(inner) => sender_address.contains(&inner.calculate_contract_address()),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        test_utils::rpc_test_setup,
        versions::user::{
            v0_10_0::{StarknetWsRpcApiV0_10_0Client, StarknetWsRpcApiV0_10_0Server},
            v0_9_0::{StarknetWsRpcApiV0_9_0Client, StarknetWsRpcApiV0_9_0Server},
        },
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
    use mp_rpc::v0_10_0::{
        FinalityStatus as FinalityStatusV10, TxnFinalityStatus as TxnFinalityStatusV10,
        TxnReceiptWithBlockInfo as TxnReceiptWithBlockInfoV10,
    };
    use mp_rpc::v0_9_0::{FinalityStatus, TxnFinalityStatus, TxnReceiptWithBlockInfo};
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction as MpTransaction};
    use serde_json::Value;
    use std::time::Duration;

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("0x1234");
    const OTHER_SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("0x5678");

    fn transaction_with_receipt(sender_address: Felt, transaction_hash: Felt) -> TransactionWithReceipt {
        TransactionWithReceipt {
            transaction: MpTransaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                contract_address: sender_address,
                ..Default::default()
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                messages_sent: vec![],
                events: vec![],
                execution_resources: ExecutionResources::default(),
                execution_result: ExecutionResult::Succeeded,
            }),
        }
    }

    async fn start_v0_9_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn start_v0_10_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let handle = server.start(StarknetWsRpcApiV0_10_0Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_new_transaction_receipts(
        client: &jsonrpsee::ws_client::WsClient,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_0_subscribeNewTransactionReceipts",
            ObjectParams::new(),
            "starknet_V0_10_0_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_0_subscribeNewTransactionReceipts")
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_preconfirmed_filter_and_sender() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_v0_9_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_9_0Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![FinalityStatus::PreConfirmed]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x4242");
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    block_number: 0,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: transaction_with_receipt(SENDER_ADDRESS, transaction_hash),
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
            .expect("Timed out waiting for receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        assert_eq!(
            serde_json::to_value(&item).expect("Failed to serialize receipt item")["result"],
            serde_json::to_value(TxnReceiptWithBlockInfo {
                transaction_receipt: transaction_with_receipt(SENDER_ADDRESS, transaction_hash)
                    .receipt
                    .to_rpc_v0_9(TxnFinalityStatus::PreConfirmed),
                block_hash: None,
                block_number: 0,
            })
            .expect("Failed to serialize expected receipt")
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_confirmed_filter_v0_10() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_v0_10_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![FinalityStatusV10::AcceptedOnL2]),
            None,
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x5151");
        let tx = transaction_with_receipt(OTHER_SENDER_ADDRESS, transaction_hash);
        let block_result = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![tx.clone()],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block");

        let expected_block_hash = block_result.block_hash;

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        assert_eq!(
            serde_json::to_value(&item).expect("Failed to serialize receipt item")["result"],
            serde_json::to_value(TxnReceiptWithBlockInfoV10 {
                transaction_receipt: tx.receipt.to_rpc_v0_10(TxnFinalityStatusV10::L2),
                block_hash: Some(expected_block_hash),
                block_number: 0,
            })
            .expect("Failed to serialize expected receipt")
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_reorg_then_resume_v0_10() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_v0_10_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let block_0_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block 0")
            .block_hash;
        let block_1_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 1,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block 1")
            .block_hash;

        let mut sub = raw_subscribe_new_transaction_receipts(&client).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_10_0::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_1_hash,
                ending_block_number: 1,
            })
            .expect("Failed to serialize expected reorg notification")
        );

        let transaction_hash = Felt::from_hex_unchecked("0xa1a1");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
        let new_block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 1,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![tx.clone()],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store replacement confirmed block 1")
            .block_hash;

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement receipt");
        let item: super::super::SubscriptionItem<TxnReceiptWithBlockInfoV10> =
            serde_json::from_value(next).expect("Failed to deserialize replacement receipt item");

        assert_eq!(
            item.result,
            TxnReceiptWithBlockInfoV10 {
                transaction_receipt: tx.receipt.to_rpc_v0_10(TxnFinalityStatusV10::L2),
                block_hash: Some(new_block_hash),
                block_number: 1,
            }
        );
    }
}
