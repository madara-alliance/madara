use crate::errors::{ErrorExtWs, OptionExtWs};

#[cfg(test)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(test))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300); // 5min

pub async fn subscribe_pending_transactions(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_details: bool,
    sender_address: Vec<starknet_types_core::felt::Felt>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = if sender_address.len() as u64 <= super::ADDRESS_FILTER_LIMIT {
        subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?
    } else {
        return Ok(subscription_sink.reject(crate::errors::StarknetWsApiError::TooManyAddressesInFilter).await);
    };

    let mut channel = starknet.backend.subscribe_pending_block();
    let sender_address = sender_address.into_iter().collect::<std::collections::HashSet<_>>();

    loop {
        tokio::time::timeout(TIMEOUT, async {
            let pending_info = std::sync::Arc::clone(&channel.borrow_and_update());
            let mut pending_txs = pending_info.tx_hashes.iter().peekable();
            let (block, _) = match pending_txs.peek() {
                Some(tx_hash) => starknet
                    .backend
                    .find_tx_hash_block(tx_hash)
                    .or_else_internal_server_error(|| {
                        format!("SubscribePendingTransactions failed to retrieve block at tx {tx_hash:#x}")
                    })?
                    .ok_or_else_internal_server_error(|| {
                        format!("SubscribePendingTransactions failed to retrieve block at tx {tx_hash:#x}")
                    })?,
                None => {
                    return channel.changed().await.or_internal_server_error("Error waiting for watch channel update")
                }
            };

            for (tx, hash) in block.inner.transactions.into_iter().zip(pending_txs) {
                let tx = match tx {
                    mp_transactions::Transaction::Invoke(ref inner)
                        if sender_address.contains(inner.sender_address()) =>
                    {
                        tx
                    }
                    mp_transactions::Transaction::Declare(ref inner)
                        if sender_address.contains(inner.sender_address()) =>
                    {
                        tx
                    }
                    mp_transactions::Transaction::DeployAccount(ref inner)
                        if sender_address.contains(inner.sender_address()) =>
                    {
                        tx
                    }
                    _ => continue,
                };

                let tx_info = if transaction_details {
                    mp_rpc::v0_8_1::PendingTxnInfo::Full(tx.into())
                } else {
                    mp_rpc::v0_8_1::PendingTxnInfo::Hash(*hash)
                };

                let msg = jsonrpsee::SubscriptionMessage::from_json(&tx_info).or_else_internal_server_error(|| {
                    format!("SubscribePendingTransactions failed to create response message at tx {hash:#x}")
                })?;

                sink.send(msg).await.or_else_internal_server_error(|| {
                    format!("SubscribePendingTransactions failed to respond to websocket request at tx {hash:#x}")
                })?;
            }

            channel.changed().await.or_internal_server_error("Error waiting for watch channel update")
        })
        .await
        .or_internal_server_error("SubscribePendingTransactions timed out")??;
    }
}

#[cfg(test)]
mod test {
    use crate::{
        versions::user::v0_8_0::{StarknetWsRpcApiV0_8_0Client, StarknetWsRpcApiV0_8_0Server},
        Starknet,
    };

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const SENDER_ADDRESS: starknet_types_core::felt::Felt = starknet_types_core::felt::Felt::from_hex_unchecked("feed");

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
    fn tx(
        #[default(starknet_types_core::felt::Felt::ZERO)] sender_address: starknet_types_core::felt::Felt,
    ) -> mp_block::TransactionWithReceipt {
        static HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let ordering = std::sync::atomic::Ordering::AcqRel;
        let transaction_hash = HASH.fetch_add(1, ordering).into();

        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0 { contract_address: sender_address, ..Default::default() },
            )),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash,
                ..Default::default()
            }),
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
    async fn subscribe_pending_transactions_ok_hash(
        _logs: (),
        starknet: Starknet,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(tx)]
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
    async fn subscribe_pending_transactions_ok_details(
        _logs: (),
        starknet: Starknet,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(tx)]
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
    async fn subscribe_pending_transactions_ok_after(
        _logs: (),
        starknet: Starknet,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(tx)]
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

        let transaction_details = false;
        let mut sub = client
            .subscribe_pending_transactions(transaction_details, vec![SENDER_ADDRESS])
            .await
            .expect("Failed subscription");
        backend.store_pending_block(pending).expect("Failed to store pending block");

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
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        #[allow(unused)]
        tx_1: mp_block::TransactionWithReceipt,
        #[from(tx)]
        #[with(SENDER_ADDRESS)]
        #[allow(unused)]
        tx_2: mp_block::TransactionWithReceipt,
        #[from(tx)]
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
