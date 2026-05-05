use mp_rpc::v0_10_2::{BlockId, BlockTag, FinalityStatus, SubscriptionTag, TxnStatusWithoutL1};
use starknet_types_core::felt::Felt;

use crate::errors::StarknetWsApiError;
use crate::versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server;

use super::starknet_unsubscribe::*;
use super::subscribe_events::subscribe_events;
use super::subscribe_new_heads::subscribe_new_heads;
use super::subscribe_new_transaction_receipts::subscribe_new_transaction_receipts_with_reorg;
use super::subscribe_new_transactions::subscribe_new_transactions_with_reorg;
use super::subscribe_transaction_status::subscribe_transaction_status;

#[jsonrpsee::core::async_trait]
#[allow(unused)]
impl StarknetWsRpcApiV0_10_2Server for crate::Starknet {
    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block_id: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_new_heads(self, subscription_sink, block_id.unwrap_or(BlockId::Tag(BlockTag::Latest))).await?)
    }

    async fn subscribe_events(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        from_address: Option<mp_rpc::v0_10_2::AddressFilter>,
        keys: Option<Vec<Vec<Felt>>>,
        block_id: Option<BlockId>,
        finality_status: Option<FinalityStatus>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_events(self, subscription_sink, from_address, keys, block_id, finality_status).await?)
    }

    async fn subscribe_transaction_status(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_transaction_status(self, subscription_sink, transaction_hash).await?)
    }

    async fn subscribe_new_transactions(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        finality_status: Option<Vec<TxnStatusWithoutL1>>,
        sender_address: Option<Vec<Felt>>,
        tags: Option<Vec<SubscriptionTag>>,
    ) -> jsonrpsee::core::SubscriptionResult {
        Ok(subscribe_new_transactions_with_reorg(self, subscription_sink, finality_status, sender_address, tags)
            .await?)
    }

    async fn subscribe_new_transaction_receipts(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        finality_status: Option<Vec<FinalityStatus>>,
        sender_address: Option<Vec<Felt>>,
    ) -> jsonrpsee::core::SubscriptionResult {
        if sender_address.as_ref().map_or(0, Vec::len) as u64 > super::ADDRESS_FILTER_LIMIT {
            subscription_sink.reject(StarknetWsApiError::TooManyAddressesInFilter).await;
            return Ok(());
        }

        Ok(subscribe_new_transaction_receipts_with_reorg(self, subscription_sink, finality_status, sender_address)
            .await?)
    }

    async fn starknet_unsubscribe(&self, subscription_id: String) -> jsonrpsee::core::RpcResult<bool> {
        Ok(starknet_unsubscribe(self, subscription_id).await?)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        test_utils::{rpc_test_setup, TestTxStatusWatcher},
        versions::user::v0_10_2::{
            methods::ws::SubscriptionItem, StarknetWsRpcApiV0_10_2Client, StarknetWsRpcApiV0_10_2Server,
        },
        Starknet,
    };
    use assert_matches::assert_matches;
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
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction as MpTransaction};
    use serde_json::Value;
    use std::time::Duration;

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("0x1234");
    const OTHER_SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("0x5678");
    const TX_HASH: Felt = Felt::from_hex_unchecked("0x3ccaabf599097d1965e1ef8317b830e76eb681016722c9364ed6e59f3252908");

    fn add_block_at_with_hash(
        backend: &std::sync::Arc<mc_db::MadaraBackend>,
        n: u64,
    ) -> (Felt, mp_rpc::v0_10_2::BlockHeader) {
        let block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: n, ..Default::default() },
                    state_diff: mp_state_update::StateDiff::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .expect("Storing block")
            .block_hash;

        let header = backend
            .block_view_on_confirmed(n)
            .expect("Retrieving block view")
            .get_block_info()
            .expect("Retrieving block info")
            .to_rpc_v0_10();

        (block_hash, header)
    }

    fn add_block_at(backend: &std::sync::Arc<mc_db::MadaraBackend>, n: u64) -> mp_rpc::v0_10_2::BlockHeader {
        add_block_at_with_hash(backend, n).1
    }

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

    async fn start_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder().build(SERVER_ADDR).await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_10_2Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_new_heads(
        client: &jsonrpsee::ws_client::WsClient,
        block_id: BlockId,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        params.insert("block_id", block_id).expect("Building subscribeNewHeads params");
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_2_subscribeNewHeads",
            params,
            "starknet_V0_10_2_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_2_subscribeNewHeads")
    }

    async fn raw_subscribe_transaction_status(
        client: &jsonrpsee::ws_client::WsClient,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        params.insert("transaction_hash", transaction_hash).expect("Building subscribeTransactionStatus params");
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_2_subscribeTransactionStatus",
            params,
            "starknet_V0_10_2_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_2_subscribeTransactionStatus")
    }

    async fn raw_subscribe_new_transaction_receipts(
        client: &jsonrpsee::ws_client::WsClient,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_2_subscribeNewTransactionReceipts",
            ObjectParams::new(),
            "starknet_V0_10_2_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_2_subscribeNewTransactionReceipts")
    }

    #[tokio::test]
    async fn subscribe_new_heads_defaults_to_latest_when_block_id_missing_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let expected = add_block_at(&backend, 0);
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, None)
            .await
            .expect("starknet_subscribeNewHeads");

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for block header")
            .expect("Waiting for block header")
            .expect("Waiting for block header");

        assert_eq!(next.result, expected);
    }

    #[tokio::test]
    async fn subscribe_new_heads_future_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Number(1)))
            .await
            .expect("starknet_subscribeNewHeads");

        let expected = add_block_at(&backend, 1);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for future block header")
            .expect("Waiting for block header")
            .expect("Waiting for block header");

        assert_eq!(next.result, expected);
    }

    #[tokio::test]
    async fn subscribe_new_heads_err_pending_v0_10_2() {
        let (_backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub =
            StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Tag(BlockTag::PreConfirmed)))
                .await
                .expect("starknet_subscribeNewHeads");

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    async fn subscribe_new_heads_unsubscribe_uses_string_id_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let _header = add_block_at(&backend, 0);
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, None)
            .await
            .expect("starknet_subscribeNewHeads");

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for block header")
            .expect("Waiting for block header")
            .expect("Waiting for block header");

        assert!(next.subscription_id.parse::<u64>().is_ok(), "subscription_id should be a numeric string");
        StarknetWsRpcApiV0_10_2Client::starknet_unsubscribe(&client, next.subscription_id)
            .await
            .expect("Failed to close subscription");

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    async fn subscribe_new_heads_reorg_then_resume_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
        let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);
        let (block_2_hash, _block_2) = add_block_at_with_hash(&backend, 2);

        let mut sub = raw_subscribe_new_heads(&client, BlockId::Number(3)).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_2_hash,
                ending_block_number: 2,
            })
            .expect("Failed to serialize expected reorg notification")
        );

        let (_new_block_1_hash, new_block_1) = add_block_at_with_hash(&backend, 1);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement head")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement head");
        let item: SubscriptionItem<mp_rpc::v0_10_2::BlockHeader> =
            serde_json::from_value(next).expect("Failed to deserialize block header item");

        assert_eq!(item.result, new_block_1);
    }

    #[tokio::test]
    async fn subscribe_new_heads_reorg_during_backfill_v0_10_2() {
        const BACKFILL_BLOCKS: u64 = 1_025;
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut block_0_hash = Felt::ZERO;
        let mut block_1_hash = Felt::ZERO;
        let mut previous_head_hash = Felt::ZERO;
        for n in 0..BACKFILL_BLOCKS {
            let (block_hash, _header) = add_block_at_with_hash(&backend, n);
            if n == 0 {
                block_0_hash = block_hash;
            } else if n == 1 {
                block_1_hash = block_hash;
            }
            previous_head_hash = block_hash;
        }

        let mut sub = raw_subscribe_new_heads(&client, BlockId::Number(0)).await;
        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let expected_reorg = serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
            starting_block_hash: block_1_hash,
            starting_block_number: 1,
            ending_block_hash: previous_head_hash,
            ending_block_number: BACKFILL_BLOCKS - 1,
        })
        .expect("Failed to serialize expected reorg notification");

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let next = sub
                    .next()
                    .await
                    .expect("Subscription closed unexpectedly")
                    .expect("Failed to retrieve backfill item");

                if next == expected_reorg {
                    break;
                }
            }
        })
        .await
        .expect("Timed out waiting for reorg notification after replay backfill");

        let (_replacement_hash, replacement_head) = add_block_at_with_hash(&backend, 1);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement head")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement head");
        let item: SubscriptionItem<mp_rpc::v0_10_2::BlockHeader> =
            serde_json::from_value(next).expect("Failed to deserialize replacement head");

        assert_eq!(item.result, replacement_head);
    }

    #[tokio::test]
    async fn unsubscribe_rejects_invalid_string_id_v0_10_2() {
        let (_backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let err = StarknetWsRpcApiV0_10_2Client::starknet_unsubscribe(&client, "not-a-number".into())
            .await
            .expect_err("unsubscribe should fail");

        assert_matches!(
            err,
            jsonrpsee::core::client::error::Error::Call(err) => {
                assert_eq!(err, crate::StarknetRpcApiError::InvalidSubscriptionId.into());
            }
        );
    }

    #[tokio::test]
    async fn subscribe_transaction_status_received_before_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        watcher.set_status(Some(crate::TxStatusSnapshot::Received));
        starknet.set_tx_status_watcher(Some(watcher));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve status");

        assert_eq!(item.result, mp_rpc::v0_10_2::TxnStatus::Received);
    }

    #[tokio::test]
    async fn subscribe_transaction_status_full_flow_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");

        watcher.set_status(Some(crate::TxStatusSnapshot::Received));
        let first = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for received status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve received status");
        assert_eq!(first.result, mp_rpc::v0_10_2::TxnStatus::Received);

        watcher.set_status(Some(crate::TxStatusSnapshot::Candidate));
        let second = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for candidate status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve candidate status");
        assert_eq!(second.result, mp_rpc::v0_10_2::TxnStatus::Candidate);

        watcher.set_status(Some(crate::TxStatusSnapshot::PreConfirmed));
        let third = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for pre-confirmed status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve pre-confirmed status");
        assert_eq!(third.result, mp_rpc::v0_10_2::TxnStatus::PreConfirmed);

        watcher.set_status(Some(crate::TxStatusSnapshot::AcceptedOnL2));
        let fourth = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for L2 status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve L2 status");
        assert_eq!(fourth.result, mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2);

        watcher.set_status(Some(crate::TxStatusSnapshot::AcceptedOnL1));
        let fifth = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for L1 status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve L1 status");
        assert_eq!(fifth.result, mp_rpc::v0_10_2::TxnStatus::AcceptedOnL1);
    }

    #[tokio::test]
    async fn subscribe_transaction_status_unsubscribe_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");
        watcher.set_status(Some(crate::TxStatusSnapshot::Received));

        let subscription_id =
            match tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status")
            {
                Some(Ok(SubscriptionItem { subscription_id, result: status })) => {
                    assert_eq!(status, mp_rpc::v0_10_2::TxnStatus::Received);
                    subscription_id
                }
                other => panic!("Unexpected subscription result: {other:?}"),
            };

        StarknetWsRpcApiV0_10_2Client::starknet_unsubscribe(&client, subscription_id)
            .await
            .expect("Failed to close subscription");
        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    async fn subscribe_transaction_status_reorg_notification_v0_10_2() {
        let (backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
        let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);

        let mut sub = raw_subscribe_transaction_status(&client, TX_HASH).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_1_hash,
                ending_block_number: 1,
            })
            .expect("Failed to serialize expected reorg notification")
        );
    }

    #[tokio::test]
    async fn subscribe_transaction_status_watcher_close_ends_subscription_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");
        watcher.close();

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for watcher-close stream termination");
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_confirmed_filter_and_sender_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![FinalityStatus::AcceptedOnL2]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x4242");
        let block_info = backend.write_access().add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![
                    transaction_with_receipt(OTHER_SENDER_ADDRESS, Felt::from_hex_unchecked("0x4141")),
                    transaction_with_receipt(SENDER_ADDRESS, transaction_hash),
                ],
                events: vec![],
            },
            &[],
            true,
        );
        let block_hash = block_info.expect("Failed to store confirmed block");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        assert_eq!(
            item.result,
            mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
                transaction_receipt: transaction_with_receipt(SENDER_ADDRESS, transaction_hash)
                    .receipt
                    .to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L2),
                block_hash: Some(block_hash.block_hash),
                block_number: 0,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_preconfirmed_filter_and_sender_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![FinalityStatus::PreConfirmed]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x4343");
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
            item.result,
            mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
                transaction_receipt: transaction_with_receipt(SENDER_ADDRESS, transaction_hash)
                    .receipt
                    .to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::PreConfirmed),
                block_hash: None,
                block_number: 0,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_reorg_then_resume_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
        let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);

        let mut sub = raw_subscribe_new_transaction_receipts(&client).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
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
            .expect("Failed to store replacement confirmed block")
            .block_hash;

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement receipt");
        let item: SubscriptionItem<mp_rpc::v0_10_2::TxnReceiptWithBlockInfo> =
            serde_json::from_value(next).expect("Failed to deserialize replacement receipt item");

        assert_eq!(
            item.result,
            mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
                transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L2),
                block_hash: Some(new_block_hash),
                block_number: 1,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_rejects_too_many_sender_addresses_v0_10_2() {
        let (_backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let size = super::super::ADDRESS_FILTER_LIMIT as usize + 1;
        let err = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![FinalityStatus::AcceptedOnL2]),
            Some(vec![SENDER_ADDRESS; size]),
        )
        .await
        .expect_err("Subscription should fail");

        assert_matches!(
            err,
            jsonrpsee::core::client::error::Error::Call(err) => {
                assert_eq!(err, crate::errors::StarknetWsApiError::TooManyAddressesInFilter.into());
            }
        );
    }
}
