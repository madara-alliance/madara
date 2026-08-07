use mp_rpc::v0_10_2::{BlockId, BlockTag, FinalityStatus, SubscriptionTag, TxnStatusWithoutL1};
use starknet_types_core::felt::Felt;

use crate::versions::user::v0_10_0::methods::ws::starknet_unsubscribe::starknet_unsubscribe;
use crate::versions::user::v0_10_0::methods::ws::subscribe_new_transaction_receipts::subscribe_new_transaction_receipts_with_reorg;
use crate::versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server;

use super::subscribe_events::subscribe_events;
use super::subscribe_new_heads::subscribe_new_heads;
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
        versions::user::{
            v0_10_0::{StarknetWsRpcApiV0_10_0Client, StarknetWsRpcApiV0_10_0Server},
            v0_10_2::{StarknetWsRpcApiV0_10_2Client, StarknetWsRpcApiV0_10_2Server},
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
        let server = jsonrpsee::server::Server::builder()
            .max_connections(1_024)
            .set_id_provider(crate::StarknetSubscriptionIdProvider::default())
            .build(SERVER_ADDR)
            .await
            .expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_10_2Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn start_server_v0_10_0(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder()
            .max_connections(1_024)
            .set_id_provider(crate::StarknetSubscriptionIdProvider::default())
            .build(SERVER_ADDR)
            .await
            .expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_10_0Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_new_heads(
        client: &jsonrpsee::ws_client::WsClient,
        block_id: BlockId,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        params.insert("block_id", block_id).expect("Building subscribeNewHeads params");
        raw_subscribe(client, "starknet_V0_10_2_subscribeNewHeads", params).await
    }

    async fn raw_subscribe_transaction_status(
        client: &jsonrpsee::ws_client::WsClient,
        transaction_hash: Felt,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        params.insert("transaction_hash", transaction_hash).expect("Building subscribeTransactionStatus params");
        raw_subscribe(client, "starknet_V0_10_2_subscribeTransactionStatus", params).await
    }

    async fn raw_subscribe_new_transaction_receipts(
        client: &jsonrpsee::ws_client::WsClient,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        raw_subscribe_new_transaction_receipts_with_params(client, ObjectParams::new()).await
    }

    async fn raw_subscribe_new_transaction_receipts_with_params(
        client: &jsonrpsee::ws_client::WsClient,
        params: ObjectParams,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        raw_subscribe(client, "starknet_V0_10_2_subscribeNewTransactionReceipts", params).await
    }

    async fn raw_subscribe(
        client: &jsonrpsee::ws_client::WsClient,
        method: &'static str,
        params: ObjectParams,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        SubscriptionClientT::subscribe(client, method, params, "starknet_V0_10_2_unsubscribe").await.expect(method)
    }

    fn expected_txn_status(finality_status: mp_rpc::v0_10_2::TxnStatus) -> mp_rpc::v0_10_2::NewTxnStatus {
        mp_rpc::v0_10_2::NewTxnStatus {
            transaction_hash: TX_HASH,
            status: mp_rpc::v0_10_2::WsTxnStatusResult {
                execution_status: None,
                finality_status,
                failure_reason: None,
            },
        }
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

        assert_eq!(next, expected);
    }

    #[tokio::test]
    async fn subscribe_new_heads_defaults_to_latest_when_block_id_missing_v0_10_0() {
        let (backend, starknet) = rpc_test_setup();
        let expected = add_block_at(&backend, 0);
        let (_handle, server_url) = start_server_v0_10_0(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_heads(&client, None)
            .await
            .expect("starknet_subscribeNewHeads");

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for block header")
            .expect("Waiting for block header")
            .expect("Waiting for block header");

        let item = serde_json::to_value(next).expect("Serializing v0.10.0 header item");
        assert_eq!(item, serde_json::to_value(expected).expect("Serializing expected header"));
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

        assert_eq!(next, expected);
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
        tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for block header")
            .expect("Waiting for block header")
            .expect("Waiting for block header");

        StarknetWsRpcApiV0_10_2Client::starknet_unsubscribe(&client, "0".into())
            .await
            .expect("Failed to close subscription");

        assert!(sub.next().await.is_none());
    }

    #[test]
    fn subscription_id_provider_returns_strings() {
        use jsonrpsee::server::IdProvider;

        let id = crate::StarknetSubscriptionIdProvider::default().next_id();
        assert_matches!(id, jsonrpsee::types::SubscriptionId::Str(id) if id.parse::<u64>().is_ok());
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
        let item: mp_rpc::v0_10_2::BlockHeader =
            serde_json::from_value(next).expect("Failed to deserialize block header item");

        assert_eq!(item, new_block_1);
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
        let item: mp_rpc::v0_10_2::BlockHeader =
            serde_json::from_value(next).expect("Failed to deserialize replacement head");

        assert_eq!(item, replacement_head);
    }

    #[tokio::test]
    async fn subscribe_new_heads_many_clients_slow_reader_and_cleanup_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let starknet_for_assert = starknet.clone();
        let (_handle, server_url) = start_server(starknet).await;
        add_block_at(&backend, 0);

        let mut next_block = 1;
        for count in [5, 50, 100, 500] {
            let mut subscribers = Vec::with_capacity(count);
            for _ in 0..count {
                let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");
                let sub =
                    StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Number(next_block)))
                        .await
                        .expect("starknet_subscribeNewHeads");
                subscribers.push((client, sub));
            }
            wait_for_active_subscriptions(&starknet_for_assert, count).await;

            let first = add_block_at(&backend, next_block);
            let second = add_block_at(&backend, next_block + 1);
            next_block += 2;

            for (_, sub) in subscribers.iter_mut().take(count - 1) {
                expect_next_head(sub, &first).await;
                let _ = expect_next_head(sub, &second).await;
            }
            expect_next_head(&mut subscribers.last_mut().expect("slow subscriber").1, &first).await;
            let _ = expect_next_head(&mut subscribers.last_mut().expect("slow subscriber").1, &second).await;

            let unsubscribe_count = count / 2;
            drop(subscribers.drain(..unsubscribe_count));
            wait_for_active_subscriptions(&starknet_for_assert, count - unsubscribe_count).await;

            let third = add_block_at(&backend, next_block);
            next_block += 1;
            for (_, sub) in subscribers.iter_mut().skip(unsubscribe_count) {
                let _ = expect_next_head(sub, &third).await;
            }

            drop(subscribers);
            wait_for_active_subscriptions(&starknet_for_assert, 0).await;
        }
    }

    async fn wait_for_active_subscriptions(starknet: &Starknet, expected: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if starknet.active_ws_subscription_count() == expected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Timed out waiting for websocket subscription cleanup");
    }

    async fn expect_next_head(
        sub: &mut jsonrpsee::core::client::Subscription<mp_rpc::v0_10_2::BlockHeader>,
        expected: &mp_rpc::v0_10_2::BlockHeader,
    ) {
        let item = tokio::time::timeout(Duration::from_secs(10), sub.next())
            .await
            .expect("Timed out waiting for block header")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve block header");
        assert_eq!(item, *expected);
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

        assert_eq!(item, expected_txn_status(mp_rpc::v0_10_2::TxnStatus::Received));
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

        for (snapshot, expected) in [
            (crate::TxStatusSnapshot::Received, mp_rpc::v0_10_2::TxnStatus::Received),
            (crate::TxStatusSnapshot::Candidate, mp_rpc::v0_10_2::TxnStatus::Candidate),
            (crate::TxStatusSnapshot::PreConfirmed, mp_rpc::v0_10_2::TxnStatus::PreConfirmed),
            (crate::TxStatusSnapshot::AcceptedOnL2, mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2),
            (crate::TxStatusSnapshot::AcceptedOnL1, mp_rpc::v0_10_2::TxnStatus::AcceptedOnL1),
        ] {
            watcher.set_status(Some(snapshot));
            let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
                .await
                .expect("Timed out waiting for status")
                .expect("Subscription closed unexpectedly")
                .expect("Failed to retrieve status");
            assert_eq!(item, expected_txn_status(expected));
        }
    }

    #[tokio::test]
    async fn subscribe_transaction_status_none_status_keeps_subscription_open_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");

        watcher.set_status(None);
        assert!(tokio::time::timeout(Duration::from_millis(100), sub.next()).await.is_err());

        watcher.set_status(Some(crate::TxStatusSnapshot::Received));
        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve status");

        assert_eq!(item, expected_txn_status(mp_rpc::v0_10_2::TxnStatus::Received));
    }

    #[tokio::test]
    async fn subscribe_transaction_status_cleanup_on_drop_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher.clone()));
        let starknet_for_assert = starknet.clone();

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");
        watcher.set_status(Some(crate::TxStatusSnapshot::Received));

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve status");
        assert_eq!(item, expected_txn_status(mp_rpc::v0_10_2::TxnStatus::Received));

        drop(sub);
        drop(client);
        wait_for_active_subscriptions(&starknet_for_assert, 0).await;
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
    async fn subscribe_transaction_status_watcher_close_does_not_emit_error_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestTxStatusWatcher::new();
        starknet.set_tx_status_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
            .await
            .expect("Failed subscription");
        watcher.close();

        assert!(tokio::time::timeout(Duration::from_millis(100), sub.next()).await.is_err());
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
            item,
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
    async fn subscribe_new_transaction_receipts_l1_confirmed_block_v0_10_2() {
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

        backend.set_latest_l1_confirmed(Some(0)).expect("Failed to set L1 confirmed block");
        let transaction_hash = Felt::from_hex_unchecked("0x4444");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
        let block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                    state_diff: Default::default(),
                    transactions: vec![tx.clone()],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block")
            .block_hash;

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        assert_eq!(
            item,
            mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
                transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L1),
                block_hash: Some(block_hash),
                block_number: 0,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_l1_confirmed_block_v0_10_0() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server_v0_10_0(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![mp_rpc::v0_10_0::FinalityStatus::AcceptedOnL2]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        backend.set_latest_l1_confirmed(Some(0)).expect("Failed to set L1 confirmed block");
        let transaction_hash = Felt::from_hex_unchecked("0x4545");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
        let block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                    state_diff: Default::default(),
                    transactions: vec![tx.clone()],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block")
            .block_hash;

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        let item = serde_json::to_value(item).expect("Failed to serialize receipt item");
        let expected = serde_json::to_value(mp_rpc::v0_10_0::TxnReceiptWithBlockInfo {
            transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_0::TxnFinalityStatus::L1),
            block_hash: Some(block_hash),
            block_number: 0,
        })
        .expect("Failed to serialize expected receipt");

        assert_eq!(item, expected);
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
            item,
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
    async fn subscribe_new_transaction_receipts_preconfirmed_append_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: 0,
                protocol_version: StarknetVersion::V0_13_2,
                ..Default::default()
            }))
            .expect("Failed to create empty preconfirmed block");

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");
        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![FinalityStatus::PreConfirmed]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x4747");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
        let executed = vec![PreconfirmedExecutedTransaction {
            transaction: tx.clone(),
            state_diff: Default::default(),
            declared_class: None,
            arrived_at: Default::default(),
            paid_fee_on_l1: None,
        }];
        backend
            .write_access()
            .append_to_preconfirmed(&executed, std::iter::empty())
            .expect("Failed to append preconfirmed transaction");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for appended preconfirmed receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        assert_eq!(
            item,
            mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
                transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::PreConfirmed),
                block_hash: None,
                block_number: 0,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_preconfirmed_append_v0_10_0() {
        let (backend, starknet) = rpc_test_setup();
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: 0,
                protocol_version: StarknetVersion::V0_13_2,
                ..Default::default()
            }))
            .expect("Failed to create empty preconfirmed block");

        let (_handle, server_url) = start_server_v0_10_0(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");
        let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_transaction_receipts(
            &client,
            Some(vec![mp_rpc::v0_10_0::FinalityStatus::PreConfirmed]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x4848");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
        let executed = vec![PreconfirmedExecutedTransaction {
            transaction: tx.clone(),
            state_diff: Default::default(),
            declared_class: None,
            arrived_at: Default::default(),
            paid_fee_on_l1: None,
        }];
        backend
            .write_access()
            .append_to_preconfirmed(&executed, std::iter::empty())
            .expect("Failed to append preconfirmed transaction");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for appended preconfirmed receipt")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve receipt");

        let item = serde_json::to_value(item).expect("Failed to serialize receipt item");
        let expected = serde_json::to_value(mp_rpc::v0_10_0::TxnReceiptWithBlockInfo {
            transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_0::TxnFinalityStatus::PreConfirmed),
            block_hash: None,
            block_number: 0,
        })
        .expect("Failed to serialize expected receipt");

        assert_eq!(item, expected);
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
        let item: mp_rpc::v0_10_2::TxnReceiptWithBlockInfo =
            serde_json::from_value(next).expect("Failed to deserialize replacement receipt item");

        assert_eq!(
            item,
            mp_rpc::v0_10_2::TxnReceiptWithBlockInfo {
                transaction_receipt: tx.receipt.to_rpc_v0_10(mp_rpc::v0_10_2::TxnFinalityStatus::L2),
                block_hash: Some(new_block_hash),
                block_number: 1,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transaction_receipts_preconfirmed_reorg_wins_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
        let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);

        let mut params = ObjectParams::new();
        params.insert("finality_status", vec![FinalityStatus::PreConfirmed]).expect("Building receipt params");
        let mut sub = raw_subscribe_new_transaction_receipts_with_params(&client, params).await;

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    block_number: 2,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: transaction_with_receipt(SENDER_ADDRESS, Felt::from_hex_unchecked("0x4646")),
                    state_diff: Default::default(),
                    declared_class: None,
                    arrived_at: Default::default(),
                    paid_fee_on_l1: None,
                }],
                vec![],
            ))
            .expect("Failed to store preconfirmed block");
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
