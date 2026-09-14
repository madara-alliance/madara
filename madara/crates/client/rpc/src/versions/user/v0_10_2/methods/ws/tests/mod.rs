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
    transaction_with_receipt_and_execution(sender_address, transaction_hash, ExecutionResult::Succeeded)
}

fn transaction_with_receipt_and_execution(
    sender_address: Felt,
    transaction_hash: Felt,
    execution_result: ExecutionResult,
) -> TransactionWithReceipt {
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
            execution_result,
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
    expected_txn_status_with_execution(finality_status, None, None)
}

fn expected_txn_status_with_execution(
    finality_status: mp_rpc::v0_10_2::TxnStatus,
    execution_status: Option<mp_rpc::v0_10_2::TxnExecutionStatus>,
    failure_reason: Option<String>,
) -> mp_rpc::v0_10_2::NewTxnStatus {
    mp_rpc::v0_10_2::NewTxnStatus {
        transaction_hash: TX_HASH,
        status: mp_rpc::v0_10_2::WsTxnStatusResult { execution_status, finality_status, failure_reason },
    }
}

/// Waits until the server reports the expected active subscription count.
/// The timeout keeps cleanup assertions bounded when a subscription leaks.
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

mod new_heads;
mod receipts;
mod transaction_status;
