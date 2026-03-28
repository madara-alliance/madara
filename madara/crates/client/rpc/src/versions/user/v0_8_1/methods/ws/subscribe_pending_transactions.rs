use crate::errors::ErrorExtWs;
use mp_transactions::validated::ValidatedTransaction;
use std::{collections::HashSet, sync::Arc};

/// Notifies the user of new transactions in the pending block which match one of several
/// `sender_address`,
///
/// The meaning of `sender_address` depends on the transaction type:
///
/// - [`Invoke`]: **sender address**.
/// - [`L1Handler`]: **L2 contract address**.
/// - [`Declare`]: **sender address**.
/// - [`Deploy`]: **deployed contract address**.
/// - [`DeployAccount`]: **deployed contract address**.
///
/// Note that it is possible to call this method on a `sender_address` which has not yet been
/// received by the node and this endpoint will send an update as soon as a tx matching that sender
/// address is received.
///
/// ## Error handling
///
/// This subscription will issue a connection refusal with [`TooManyAddressesInFilter`] if more than
/// [`ADDRESS_FILTER_LIMIT`] sender addresses are provided.
///
/// [`Invoke`]: mp_transactions::Transaction::Invoke
/// [`L1Handler`]: mp_transactions::Transaction::L1Handler
/// [`Declare`]: mp_transactions::Transaction::Declare
/// [`Deploy`]: mp_transactions::Transaction::Deploy
/// [`DeployAccount`]: mp_transactions::Transaction::DeployAccount
/// [`TooManyAddressesInFilter`]: crate::errors::StarknetWsApiError::TooManyAddressesInFilter
/// [`ADDRESS_FILTER_LIMIT`]: super::ADDRESS_FILTER_LIMIT
pub async fn subscribe_pending_transactions(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_details: bool,
    sender_address: Vec<starknet_types_core::felt::Felt>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = if sender_address.len() as u64 <= super::ADDRESS_FILTER_LIMIT {
        subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?
    } else {
        subscription_sink.reject(crate::errors::StarknetWsApiError::TooManyAddressesInFilter).await;
        return Ok(());
    };

    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;
    let sender_address = sender_address.into_iter().collect::<HashSet<_>>();
    let mut watch = starknet
        .new_transactions_watcher
        .as_ref()
        .ok_or_else(|| {
            crate::errors::StarknetWsApiError::internal_server_error(
                "SubscribePendingTransactions failed: new-transactions watcher is not configured",
            )
        })?
        .watch_new_transactions()
        .ok_or_else(|| {
            crate::errors::StarknetWsApiError::internal_server_error(
                "SubscribePendingTransactions failed to create new-transactions watcher",
            )
        })?;

    loop {
        let Some(tx) = next_matching_transaction(&sink, &ctx, &mut watch, &sender_address).await? else {
            return Ok(());
        };

        let tx_hash = tx.hash;
        let tx_info = if transaction_details {
            mp_rpc::v0_8_1::PendingTxnInfo::Full(tx.transaction.clone().to_rpc_v0_7())
        } else {
            mp_rpc::v0_8_1::PendingTxnInfo::Hash(tx_hash)
        };

        let item = super::SubscriptionItem::new(sink.subscription_id(), tx_info);
        let msg = jsonrpsee::SubscriptionMessage::from_json(&item).or_else_internal_server_error(|| {
            format!("SubscribePendingTransactions failed to create response message at tx {tx_hash:#x}")
        })?;

        sink.send(msg).await.or_else_internal_server_error(|| {
            format!("SubscribePendingTransactions failed to respond to websocket request at tx {tx_hash:#x}")
        })?;
    }
}

async fn next_matching_transaction(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    ctx: &crate::WsSubscriptionGuard,
    watch: &mut Box<dyn crate::NewTransactionsWatch + Send>,
    sender_address: &HashSet<starknet_types_core::felt::Felt>,
) -> Result<Option<Arc<ValidatedTransaction>>, crate::errors::StarknetWsApiError> {
    loop {
        let next = tokio::select! {
            _ = sink.closed() => return Ok(None),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
            next = watch.recv() => next,
        };

        let Some(tx) = next else {
            return Ok(None);
        };

        if sender_address.contains(&tx.contract_address) {
            return Ok(Some(tx));
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        test_utils::{TestNewTransactionsWatcher, TestTransactionProvider},
        versions::user::v0_8_1::{
            methods::ws::SubscriptionItem, StarknetWsRpcApiV0_8_1Client, StarknetWsRpcApiV0_8_1Server,
        },
        Starknet,
    };
    use assert_matches::assert_matches;
    use mp_chain_config::ChainConfig;
    use mp_transactions::{
        validated::{TxTimestamp, ValidatedTransaction},
        DeclareTransaction, DeclareTransactionV0, DeployAccountTransaction, DeployAccountTransactionV1,
        DeployTransaction, InvokeTransaction, InvokeTransactionV0, L1HandlerTransaction, Transaction,
    };
    use mp_utils::service::ServiceContext;
    use starknet_types_core::felt::Felt;
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering::Relaxed},
            Arc,
        },
        time::Duration,
    };

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("feed");
    const CONTRACT_ADDRESS: Felt =
        Felt::from_hex_unchecked("0x64820103001fcf57dc33ea01733a819529381f2df018c97621e4089f0f0d355");

    fn next_hash() -> Felt {
        static HASH: AtomicU64 = AtomicU64::new(1);
        HASH.fetch_add(1, Relaxed).into()
    }

    fn validated_tx(transaction: Transaction, contract_address: Felt) -> ValidatedTransaction {
        ValidatedTransaction {
            transaction,
            paid_fee_on_l1: None,
            contract_address,
            arrived_at: TxTimestamp::now(),
            declared_class: None,
            hash: next_hash(),
            charge_fee: true,
        }
    }

    fn invoke_tx(sender_address: Felt) -> ValidatedTransaction {
        validated_tx(
            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                contract_address: sender_address,
                ..Default::default()
            })),
            sender_address,
        )
    }

    fn l1_handler_tx(contract_address: Felt) -> ValidatedTransaction {
        validated_tx(
            Transaction::L1Handler(L1HandlerTransaction { contract_address, ..Default::default() }),
            contract_address,
        )
    }

    fn declare_tx(sender_address: Felt) -> ValidatedTransaction {
        validated_tx(
            Transaction::Declare(DeclareTransaction::V0(DeclareTransactionV0 { sender_address, ..Default::default() })),
            sender_address,
        )
    }

    fn deploy_tx(contract_address: Felt) -> ValidatedTransaction {
        validated_tx(Transaction::Deploy(DeployTransaction::default()), contract_address)
    }

    fn deploy_account_tx(contract_address: Felt) -> ValidatedTransaction {
        validated_tx(
            Transaction::DeployAccount(DeployAccountTransaction::V1(DeployAccountTransactionV1::default())),
            contract_address,
        )
    }

    fn starknet_with_new_transactions_watcher() -> (Starknet, Arc<TestNewTransactionsWatcher>) {
        let chain_config = Arc::new(ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let watcher = TestNewTransactionsWatcher::new();
        let mut starknet = Starknet::new(
            backend,
            Arc::new(TestTransactionProvider),
            Default::default(),
            None,
            ServiceContext::new_for_testing(),
        );
        starknet.set_new_transactions_watcher(Some(watcher.clone()));
        (starknet, watcher)
    }

    async fn ws_client(starknet: Starknet) -> (jsonrpsee::ws_client::WsClient, jsonrpsee::server::ServerHandle) {
        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = jsonrpsee::ws_client::WsClientBuilder::default()
            .build(&server_url)
            .await
            .expect("Failed to start jsonrpsee ws client");
        (client, server_handle)
    }

    #[tokio::test]
    async fn subscribe_pending_transactions_ok_hash() {
        let (starknet, watcher) = starknet_with_new_transactions_watcher();
        let tx_1 = invoke_tx(SENDER_ADDRESS);
        let tx_2 = invoke_tx(SENDER_ADDRESS);
        let tx_3 = invoke_tx(Felt::ONE);
        let (client, _server_handle) = ws_client(starknet).await;

        let mut sub =
            client.subscribe_pending_transactions(false, vec![SENDER_ADDRESS]).await.expect("Failed subscription");

        watcher.send_transaction(tx_3);
        watcher.send_transaction(tx_1.clone());
        watcher.send_transaction(tx_2.clone());

        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for tx"),
            Some(Ok(SubscriptionItem { result: tx, .. })) => {
                assert_eq!(tx, mp_rpc::v0_8_1::PendingTxnInfo::Hash(tx_1.hash));
            }
        );
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for tx"),
            Some(Ok(SubscriptionItem { result: tx, .. })) => {
                assert_eq!(tx, mp_rpc::v0_8_1::PendingTxnInfo::Hash(tx_2.hash));
            }
        );
    }

    #[tokio::test]
    async fn subscribe_pending_transactions_ok_details() {
        let (starknet, watcher) = starknet_with_new_transactions_watcher();
        let tx_1 = invoke_tx(SENDER_ADDRESS);
        let tx_2 = invoke_tx(SENDER_ADDRESS);
        let tx_3 = invoke_tx(Felt::ONE);
        let (client, _server_handle) = ws_client(starknet).await;

        let mut sub =
            client.subscribe_pending_transactions(true, vec![SENDER_ADDRESS]).await.expect("Failed subscription");

        watcher.send_transaction(tx_3);
        watcher.send_transaction(tx_1.clone());
        watcher.send_transaction(tx_2.clone());

        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for tx"),
            Some(Ok(SubscriptionItem { result: tx, .. })) => {
                assert_eq!(tx, mp_rpc::v0_8_1::PendingTxnInfo::Full(tx_1.transaction.clone().to_rpc_v0_7()));
            }
        );
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for tx"),
            Some(Ok(SubscriptionItem { result: tx, .. })) => {
                assert_eq!(tx, mp_rpc::v0_8_1::PendingTxnInfo::Full(tx_2.transaction.clone().to_rpc_v0_7()));
            }
        );
    }

    #[tokio::test]
    async fn subscribe_pending_transaction_ok_all_types() {
        let (starknet, watcher) = starknet_with_new_transactions_watcher();
        let deploy_account = deploy_account_tx(CONTRACT_ADDRESS);
        let deploy = deploy_tx(CONTRACT_ADDRESS);
        let declare = declare_tx(CONTRACT_ADDRESS);
        let l1_handler = l1_handler_tx(CONTRACT_ADDRESS);
        let invoke = invoke_tx(CONTRACT_ADDRESS);
        let (client, _server_handle) = ws_client(starknet).await;

        let mut sub =
            client.subscribe_pending_transactions(false, vec![CONTRACT_ADDRESS]).await.expect("Failed subscription");

        watcher.send_transaction(deploy_account.clone());
        watcher.send_transaction(deploy.clone());
        watcher.send_transaction(declare.clone());
        watcher.send_transaction(l1_handler.clone());
        watcher.send_transaction(invoke.clone());

        for expected_hash in [deploy_account.hash, deploy.hash, declare.hash, l1_handler.hash, invoke.hash] {
            assert_matches!(
                tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for tx"),
                Some(Ok(SubscriptionItem { result: tx, .. })) => {
                    assert_eq!(tx, mp_rpc::v0_8_1::PendingTxnInfo::Hash(expected_hash));
                }
            );
        }
    }

    #[tokio::test]
    async fn subscribe_pending_transactions_err_too_many_sender_address() {
        let (starknet, _watcher) = starknet_with_new_transactions_watcher();
        let (client, _server_handle) = ws_client(starknet).await;

        let size = super::super::ADDRESS_FILTER_LIMIT as usize + 1;
        let err = client
            .subscribe_pending_transactions(false, vec![SENDER_ADDRESS; size])
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
