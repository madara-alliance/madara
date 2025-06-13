use crate::errors::{ErrorExtWs, OptionExtWs};

#[cfg(test)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(not(test))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300); // 5min

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
/// ## DOS mitigation
///
/// To avoid a malicious attacker keeping connections open indefinitely on a nonexistent sender
/// address, this endpoint will terminate the connection after a global timeout period. This timeout
/// is reset every time a pending block is encountered which contains at least one matching
/// transaction. Essentially, this means that the connection will remain active for as long as a new
/// pending block with matching transactions is found within [`TIMEOUT`] seconds.
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

    let pending_info = std::sync::Arc::clone(&starknet.backend.latest_pending_block());
    let mut channel = starknet.backend.subscribe_pending_txs();
    let sender_address = sender_address.into_iter().collect::<std::collections::HashSet<_>>();

    if let Some(tx_hash) = pending_info.tx_hashes.iter().peekable().peek() {
        let pending = starknet
            .backend
            .find_tx_hash_block(tx_hash)
            .or_else_internal_server_error(|| {
                format!("SubscribePendingTransactions failed to retrieve pending block from db for tx {tx_hash:#x}")
            })?
            .ok_or_else_internal_server_error(|| {
                format!("SubscribePendingTransactions failed to retrieve pending block from db for tx {tx_hash:#x}")
            })?
            .0
            .into_pending()
            .ok_or_else_internal_server_error(|| {
                format!("SubscribePendingTransactions failed to retrieve pending block from db for tx {tx_hash:#x}")
            })?;

        for (tx, tx_hash) in pending.inner.transactions.iter().zip(pending_info.tx_hashes.iter()) {
            send_tx_if_matching(tx.clone(), *tx_hash, &sender_address, transaction_details, &sink).await?;
        }
    }

    loop {
        tokio::time::timeout(TIMEOUT, async {
            let tx_receipt = channel
                .recv()
                .await
                .or_internal_server_error("SubscribePendingTransactions failed to wait on pending transactions")?;

            let tx_hash = tx_receipt.receipt.transaction_hash();
            let tx = tx_receipt.transaction;
            send_tx_if_matching(tx, tx_hash, &sender_address, transaction_details, &sink).await?;

            Ok(())
        })
        .await
        .or_internal_server_error("SubscribePendingTransactions timed out")??;
    }
}

async fn send_tx_if_matching(
    tx: mp_transactions::Transaction,
    tx_hash: starknet_types_core::felt::Felt,
    sender_address: &std::collections::HashSet<starknet_types_core::felt::Felt>,
    transaction_details: bool,
    sink: &jsonrpsee::server::SubscriptionSink,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let tx = match tx {
        mp_transactions::Transaction::Invoke(ref inner) if sender_address.contains(inner.sender_address()) => tx,
        mp_transactions::Transaction::L1Handler(ref inner) if sender_address.contains(&inner.contract_address) => tx,
        mp_transactions::Transaction::Declare(ref inner) if sender_address.contains(inner.sender_address()) => tx,
        mp_transactions::Transaction::Deploy(ref inner)
            if sender_address.contains(&inner.calculate_contract_address()) =>
        {
            tx
        }
        mp_transactions::Transaction::DeployAccount(ref inner)
            if sender_address.contains(&inner.calculate_contract_address()) =>
        {
            tx
        }
        _ => return Ok(()),
    };

    let tx_info = if transaction_details {
        mp_rpc::v0_8_1::PendingTxnInfo::Full(tx.into())
    } else {
        mp_rpc::v0_8_1::PendingTxnInfo::Hash(tx_hash)
    };

    let msg = jsonrpsee::SubscriptionMessage::from_json(&tx_info).or_else_internal_server_error(|| {
        format!("SubscribePendingTransactions failed to create response message at tx {tx_hash:#x}")
    })?;

    sink.send(msg).await.or_else_internal_server_error(|| {
        format!("SubscribePendingTransactions failed to respond to websocket request at tx {tx_hash:#x}")
    })?;

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::{
        versions::user::v0_8_0::{StarknetWsRpcApiV0_8_0Client, StarknetWsRpcApiV0_8_0Server},
        Starknet,
    };

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const SENDER_ADDRESS: starknet_types_core::felt::Felt = starknet_types_core::felt::Felt::from_hex_unchecked("feed");
    const CONTRACT_ADDRESS: starknet_types_core::felt::Felt = starknet_types_core::felt::Felt::from_hex_unchecked(
        "0x64820103001fcf57dc33ea01733a819529381f2df018c97621e4089f0f0d355",
    );

    #[rstest::fixture]
    fn logs() {
        let debug = tracing_subscriber::filter::LevelFilter::DEBUG;
        let env = tracing_subscriber::EnvFilter::builder().with_default_directive(debug.into()).from_env_lossy();
        let _ = tracing_subscriber::fmt().with_test_writer().with_env_filter(env).with_line_number(true).try_init();
    }

    #[rstest::fixture]
    fn starknet() -> Starknet {
        let chain_config = std::sync::Arc::new(mp_chain_config::ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let validation = mc_submit_tx::TransactionValidatorConfig { disable_validation: true };
        let mempool = std::sync::Arc::new(mc_mempool::Mempool::new(
            std::sync::Arc::clone(&backend),
            mc_mempool::MempoolConfig::for_testing(),
        ));
        let mempool_validator = std::sync::Arc::new(mc_submit_tx::TransactionValidator::new(
            mempool,
            std::sync::Arc::clone(&backend),
            validation,
        ));
        let context = mp_utils::service::ServiceContext::new_for_testing();

        Starknet::new(backend, mempool_validator, Default::default(), context)
    }

    #[rstest::fixture]
    fn receipt() -> mp_receipt::TransactionReceipt {
        const HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let ordering = std::sync::atomic::Ordering::AcqRel;
        let transaction_hash = HASH.fetch_add(1, ordering).into();

        mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
            transaction_hash,
            ..Default::default()
        })
    }

    #[rstest::fixture]
    fn invoke(
        #[default(Default::default())] sender_address: starknet_types_core::felt::Felt,
        receipt: mp_receipt::TransactionReceipt,
    ) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0 { contract_address: sender_address, ..Default::default() },
            )),
            receipt,
        }
    }

    #[rstest::fixture]
    fn l1_handler(
        #[default(Default::default())] contract_address: starknet_types_core::felt::Felt,
        receipt: mp_receipt::TransactionReceipt,
    ) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::L1Handler(mp_transactions::L1HandlerTransaction {
                contract_address,
                ..Default::default()
            }),
            receipt,
        }
    }

    #[rstest::fixture]
    fn declare(
        #[default(Default::default())] sender_address: starknet_types_core::felt::Felt,
        receipt: mp_receipt::TransactionReceipt,
    ) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V0(
                mp_transactions::DeclareTransactionV0 { sender_address, ..Default::default() },
            )),
            receipt,
        }
    }

    #[rstest::fixture]
    fn deploy(receipt: mp_receipt::TransactionReceipt) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Deploy(mp_transactions::DeployTransaction::default()),
            receipt,
        }
    }

    #[rstest::fixture]
    fn deploy_account(receipt: mp_receipt::TransactionReceipt) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::DeployAccount(mp_transactions::DeployAccountTransaction::V1(
                mp_transactions::DeployAccountTransactionV1::default(),
            )),
            receipt,
        }
    }

    #[rstest::fixture]
    fn pending(
        #[default(Vec::new())] transactions: Vec<mp_block::TransactionWithReceipt>,
    ) -> mp_block::PendingFullBlock {
        mp_block::PendingFullBlock {
            header: Default::default(),
            state_diff: Default::default(),
            transactions,
            events: Default::default(),
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transactions_ok_hash_before(
        _logs: (),
        starknet: Starknet,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(starknet_types_core::felt::Felt::ONE)]
        #[allow(unused)]
        tx_3: mp_block::TransactionWithReceipt,
        #[from(pending)]
        #[with(vec![tx_1.clone(), tx_2.clone(), tx_3.clone()])]
        pending: mp_block::PendingFullBlock,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        backend.store_pending_block(pending).expect("Failed to store pending block");
        let transaction_details = false;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS])
            .await
            .expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, tx_1.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_1.receipt.transaction_hash());

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, tx_2.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_2.receipt.transaction_hash());

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transactions_ok_hash_after(
        _logs: (),
        starknet: Starknet,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(starknet_types_core::felt::Felt::ONE)]
        #[allow(unused)]
        tx_3: mp_block::TransactionWithReceipt,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        let transaction_details = false;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS])
            .await
            .expect("Failed subscription");

        backend.on_new_pending_tx(tx_1.clone());
        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, tx_1.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_1.receipt.transaction_hash());

        backend.on_new_pending_tx(tx_2.clone());
        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, tx_2.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_2.receipt.transaction_hash());

        backend.on_new_pending_tx(tx_3);
        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transactions_ok_details_before(
        _logs: (),
        starknet: Starknet,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(starknet_types_core::felt::Felt::ONE)]
        #[allow(unused)]
        tx_3: mp_block::TransactionWithReceipt,
        #[from(pending)]
        #[with(vec![tx_1.clone(), tx_2.clone(), tx_3.clone()])]
        pending: mp_block::PendingFullBlock,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        backend.store_pending_block(pending).expect("Failed to store pending block");
        let transaction_details = true;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS])
            .await
            .expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(tx)) => {
                assert_matches::assert_matches!(
                    tx, mp_rpc::v0_8_1::PendingTxnInfo::Full(tx) => {
                        assert_eq!(tx, tx_1.transaction.into());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_1.receipt.transaction_hash());

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(tx)) => {
                assert_matches::assert_matches!(
                    tx, mp_rpc::v0_8_1::PendingTxnInfo::Full(tx) => {
                        assert_eq!(tx, tx_2.transaction.into());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_2.receipt.transaction_hash());

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transactions_ok_details_after(
        _logs: (),
        starknet: Starknet,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(starknet_types_core::felt::Felt::ONE)]
        #[allow(unused)]
        tx_3: mp_block::TransactionWithReceipt,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        let transaction_details = true;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS])
            .await
            .expect("Failed subscription");

        backend.on_new_pending_tx(tx_1.clone());
        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(tx)) => {
                assert_matches::assert_matches!(
                    tx, mp_rpc::v0_8_1::PendingTxnInfo::Full(tx) => {
                        assert_eq!(tx, tx_1.transaction.into());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_1.receipt.transaction_hash());

        backend.on_new_pending_tx(tx_2.clone());
        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(tx)) => {
                assert_matches::assert_matches!(
                    tx, mp_rpc::v0_8_1::PendingTxnInfo::Full(tx) => {
                        assert_eq!(tx, tx_2.transaction.into());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", tx_2.receipt.transaction_hash());

        backend.on_new_pending_tx(tx_3);
        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transaction_ok_all_types(
        _logs: (),
        starknet: Starknet,
        deploy_account: mp_block::TransactionWithReceipt,
        deploy: mp_block::TransactionWithReceipt,
        #[with(CONTRACT_ADDRESS)] declare: mp_block::TransactionWithReceipt,
        #[with(CONTRACT_ADDRESS)] l1_handler: mp_block::TransactionWithReceipt,
        #[with(CONTRACT_ADDRESS)] invoke: mp_block::TransactionWithReceipt,
        #[with(vec![deploy_account.clone(), deploy.clone(), declare.clone(), l1_handler.clone(), invoke.clone()])]
        pending: mp_block::PendingFullBlock,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        backend.store_pending_block(pending).expect("Failed to store pending block");
        let transaction_details = false;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![CONTRACT_ADDRESS])
            .await
            .expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, deploy_account.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", deploy_account.receipt.transaction_hash());

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, deploy.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", deploy.receipt.transaction_hash());

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, declare.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", declare.receipt.transaction_hash());

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, l1_handler.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", l1_handler.receipt.transaction_hash());

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(hash)) => {
                assert_matches::assert_matches!(
                    hash, mp_rpc::v0_8_1::PendingTxnInfo::Hash(hash) => {
                        assert_eq!(hash, invoke.receipt.transaction_hash());
                    }
                )
            }
        );

        tracing::debug!("Received {:#x}", invoke.receipt.transaction_hash());

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transactions_err_timeout(_logs: (), starknet: Starknet) {
        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        let transaction_details = false;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS])
            .await
            .expect("Failed subscription");

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 2)]
    async fn subscribe_pending_transactions_err_too_many_sender_address(
        _logs: (),
        starknet: Starknet,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        #[allow(unused)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(SENDER_ADDRESS)]
        #[allow(unused)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(invoke)]
        #[with(starknet_types_core::felt::Felt::ONE)]
        #[allow(unused)]
        tx_3: mp_block::TransactionWithReceipt,
        #[from(pending)]
        #[with(vec![tx_1.clone(), tx_2.clone(), tx_3.clone()])]
        pending: mp_block::PendingFullBlock,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        backend.store_pending_block(pending).expect("Failed to store pending block");

        let transaction_details = false;
        let size = super::super::ADDRESS_FILTER_LIMIT as usize + 1;
        let err = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS; size])
            .await
            .expect_err("Subscription should fail");

        assert_matches::assert_matches!(
            err,
            jsonrpsee::core::client::error::Error::Call(err) => {
                assert_eq!(err, crate::errors::StarknetWsApiError::TooManyAddressesInFilter.into());
            }
        );
    }
}
