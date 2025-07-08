use crate::errors::ErrorExtWs;

/// Notifies the subscriber of updates to a transaction's status. ([specs])
///
/// Supported statuses are:
///
/// - [`Received`]: tx has been inserted into the mempool.
/// - [`AcceptedOnL2`]: tx has been saved to the pending block.
/// - [`AcceptedOnL1`]: tx has been finalized on L1.
///
/// We do not currently support the **Rejected** transaction status.
///
/// Note that it is possible to call this method on a transaction which has not yet been received by
/// the node and this endpoint will send an update as soon as the tx is received.
///
/// ## Returns
///
/// This subscription will automatically close once a transaction has reached [`AcceptedOnL1`].
///
/// [specs]: https://github.com/starkware-libs/starknet-specs/blob/a2d10fc6cbaddbe2d3cf6ace5174dd0a306f4885/api/starknet_ws_api.json#L127C5-L168C7
/// [`Received`]: mp_rpc::v0_7_1::TxnStatus::Received
/// [`AcceptedOnL2`]: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
/// [`AcceptedOnL1`]: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1
pub async fn subscribe_transaction_status(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_hash: mp_convert::Felt,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = subscription_sink
        .accept()
        .await
        .or_internal_server_error("SubscribeTransactionStatus failed to establish websocket connection")?;

    SubscriptionState::new(starknet, &sink, transaction_hash).await?.drive().await
}

/// State-machine-based transactions status discovery.
///
/// The state machine progresses through a series of legals states and transitions as defined by
/// implementors of the [`StateTransition`] trait. Each state is responsible for checking the status
/// of a single transaction state and moving on to the next state check once this has completed.
#[derive(Default)]
enum SubscriptionState<'a> {
    #[default]
    None,
    WaitReceived(StateTransitionReceived<'a>),
    WaitAcceptedOnL2(StateTransitionAcceptedOnL2<'a>),
    WaitAcceptedOnL1(StateTransitionAcceptedOnL1<'a>),
}

impl std::fmt::Debug for SubscriptionState<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::WaitReceived(..) => write!(f, "WaitReceived"),
            Self::WaitAcceptedOnL2(..) => write!(f, "WaitAcceptedOnL2"),
            Self::WaitAcceptedOnL1(..) => write!(f, "WaitAcceptedOnL1"),
        }
    }
}

impl<'a> SubscriptionState<'a> {
    /// This function is responsible for initializing the state machine.
    ///
    /// It does so by determining the initial state of the transaction, which in turn determines in
    /// which state the state machine starts.
    #[tracing::instrument(skip_all)]
    async fn new(
        starknet: &'a crate::Starknet,
        sink: &'a jsonrpsee::core::server::SubscriptionSink,
        tx_hash: mp_convert::Felt,
    ) -> Result<Self, crate::errors::StarknetWsApiError> {
        let common = StateTransitionCommon { starknet, sink, tx_hash };

        // **FOOTGUN!** 💥
        //
        // We subscribe to each channel before running status checks against the transaction to
        // avoid missing any updates.
        let channel_mempool = common.starknet.add_transaction_provider.subscribe_new_transactions().await;
        let channel_pending_tx = common.starknet.backend.subscribe_pending_txs();
        let channel_confirmed = common.starknet.backend.subscribe_last_block_on_l1();

        let block_info = starknet.backend.find_tx_hash_block_info(&tx_hash).or_else_internal_server_error(|| {
            format!("SubscribeTransactionStatus failed to retrieve block info for tx {tx_hash:#x}")
        })?;

        match block_info {
            Some((mp_block::MadaraMaybePendingBlockInfo::Pending(block_info), _idx)) => {
                // Tx has not yet been accepted on L1 but is included on L2, hence it is marked
                // as accepted on L2. We wait for it to be accepted on L1
                let block_number = block_info.header.parent_block_number.map(|n| n + 1).unwrap_or(0);
                tracing::debug!("WaitAcceptedOnL1");
                common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                Ok(Self::WaitAcceptedOnL1(StateTransitionAcceptedOnL1 { common, block_number, channel_confirmed }))
            }
            Some((mp_block::MadaraMaybePendingBlockInfo::NotPending(block_info), _idx)) => {
                let block_number = block_info.header.block_number;
                let confirmed =
                    common.starknet.backend.get_l1_last_confirmed_block().or_internal_server_error(
                        "SubscribeTransactionStatus failed to retrieving last confirmed block",
                    )?;

                // Tx has been accepted on L1, hence it is marked as such. This is the final
                // stage of the transaction so the state machine is put in its end state.
                if confirmed.is_some_and(|n| block_number <= n) {
                    tracing::debug!("WaitNone");
                    common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1).await?;
                    Ok(Self::None)
                }
                // Tx has not yet been accepted on L1 but is included on L2, hence it is marked
                // as accepted on L2. We wait for it to be accepted on L1
                else {
                    tracing::debug!("WaitAcceptedOnL1");
                    common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                    Ok(Self::WaitAcceptedOnL1(StateTransitionAcceptedOnL1 { common, block_number, channel_confirmed }))
                }
            }
            None => {
                // Local mempool is the only AddTransactionProvider which allows us to inspect the state
                // of received transactions. For other providers (such as when forwarding to a remote
                // gateway), we default to assuming that the transaction has been received and wait for
                // it to be accepted on L2.
                let received = common
                    .starknet
                    .add_transaction_provider
                    .received_transaction(common.tx_hash)
                    .await
                    .unwrap_or_default();
                match channel_mempool {
                    // Tx has not been received yet, we wait for it to be received in the mempool
                    Some(channel_mempool) if !received => {
                        tracing::debug!("WaitReceived");
                        Ok(Self::WaitReceived(StateTransitionReceived { common, channel_mempool }))
                    }
                    // Tx has been received or we are forwarding to a remote gateway (in which case we
                    // assume the transaction has been received). We wait for it to be accepted on L2.
                    _ => {
                        tracing::debug!("WaitAcceptedOnL2");
                        common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Received).await?;
                        Ok(Self::WaitAcceptedOnL2(StateTransitionAcceptedOnL2 { common, channel_pending_tx }))
                    }
                }
            }
        }
    }

    /// This function is responsible for driving the state machine to completion. It is also
    /// responsible for sending status updates back to the client. Status updates are not the
    /// responsibility of the [`StateTransition`] implementors and are instead centralized here.
    ///
    /// ## Legal state transitions
    ///
    /// ```text
    ///
    ///             ┌────┐
    ///          ┌─►│None├─────────────────────────────────────────────────────────┐
    ///          │  └────┘                                                         │
    ///          │                                                                 │
    ///          │                                                                 │
    /// ┌─────┐  │  ┌────────────┐                             ┌────────────────┐  └─►┌───┐
    /// │START├──┼─►│WaitReceived│┬────────────────────────┬──►│WaitAcceptedOnL1│────►│END│
    /// └─────┘  │  └────────────┘│                        │   └────────────────┘     └───┘
    ///          │                │                        ▲
    ///          │                └───►┌────────────────┐  │
    ///          └────────────────────►│WaitAcceptedOnL2│──┘
    ///                                └────────────────┘
    ///
    /// ```
    #[tracing::instrument()]
    async fn drive(&mut self) -> Result<(), crate::errors::StarknetWsApiError> {
        loop {
            match std::mem::take(self) {
                Self::None => return Ok(()),
                Self::WaitReceived(state) => {
                    let s = tokio::select! {
                        _ = state.common.sink.closed() => break Ok(()),
                        s = state.transition() => s?,
                    };
                    match s {
                        TransitionMatrixReceived::WaitAcceptedOnL2(s) => {
                            s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Received).await?;
                            *self = Self::WaitAcceptedOnL2(s);
                        }
                        TransitionMatrixReceived::WaitAcceptedOnL1(s) => {
                            s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                            *self = Self::WaitAcceptedOnL1(s);
                        }
                    }
                }
                Self::WaitAcceptedOnL2(state) => {
                    let s = tokio::select! {
                        _ = state.common.sink.closed() => break Ok(()),
                        s = state.transition() => s?,
                    };
                    s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                    *self = Self::WaitAcceptedOnL1(s);
                }
                Self::WaitAcceptedOnL1(state) => {
                    let s = tokio::select! {
                        _ = state.common.sink.closed() => break Ok(()),
                        s = state.transition() => s?,
                    };
                    s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1).await?;
                    break Ok(());
                }
            }
        }
    }
}

struct StateTransitionCommon<'a> {
    starknet: &'a crate::Starknet,
    sink: &'a jsonrpsee::core::server::SubscriptionSink,
    tx_hash: mp_convert::Felt,
}
struct StateTransitionReceived<'a> {
    common: StateTransitionCommon<'a>,
    channel_mempool: tokio::sync::broadcast::Receiver<mp_convert::Felt>,
}
struct StateTransitionAcceptedOnL2<'a> {
    common: StateTransitionCommon<'a>,
    channel_pending_tx: mc_db::PendingTxsReceiver,
}
struct StateTransitionAcceptedOnL1<'a> {
    common: StateTransitionCommon<'a>,
    block_number: u64,
    channel_confirmed: mc_db::LastBlockOnL1Receiver,
}
struct StateTransitionEnd<'a> {
    common: StateTransitionCommon<'a>,
}
enum TransitionMatrixReceived<'a> {
    WaitAcceptedOnL2(StateTransitionAcceptedOnL2<'a>),
    WaitAcceptedOnL1(StateTransitionAcceptedOnL1<'a>),
}

impl StateTransitionCommon<'_> {
    async fn send_txn_status(
        &self,
        status: mp_rpc::v0_7_1::TxnStatus,
    ) -> Result<(), crate::errors::StarknetWsApiError> {
        let txn_status = mp_rpc::v0_8_1::TxnStatus { transaction_hash: self.tx_hash, status };
        let msg = jsonrpsee::SubscriptionMessage::from_json(&txn_status).or_else_internal_server_error(|| {
            format!("SubscribeTransactionStatus failed to create response for tx hash {:#x}", self.tx_hash)
        })?;

        self.sink
            .send(msg)
            .await
            .or_internal_server_error("SubscribeTransactionStatus failed to respond to websocket request")
    }
}

trait StateTransition: Sized {
    type TransitionTo;

    async fn transition(self) -> Result<Self::TransitionTo, crate::errors::StarknetWsApiError>;
}
impl<'a> StateTransition for StateTransitionReceived<'a> {
    type TransitionTo = TransitionMatrixReceived<'a>;

    async fn transition(self) -> Result<Self::TransitionTo, crate::errors::StarknetWsApiError> {
        let Self { common, mut channel_mempool, .. } = self;

        let channel_confirmed = common.starknet.backend.subscribe_last_block_on_l1();
        let tx_hash = &common.tx_hash;

        loop {
            // **FOOTGUN!** 💥
            //
            // We delay the pending tx subscription as much as possible so that `WaitAcceptedOnL2`
            // will only have to check at most the latest few transactions. If we subscribed to this
            // channel at the start of the function, it would be receiving ALL txs from then
            // until the transaction was included into the pending block and `WaitAcceptedOnL2`
            // would have to check them all!
            let channel_pending_tx = common.starknet.backend.subscribe_pending_txs();
            match channel_mempool.recv().await {
                Ok(hash) => {
                    if &hash == tx_hash {
                        let transition = StateTransitionAcceptedOnL2 { common, channel_pending_tx };
                        let transition = Self::TransitionTo::WaitAcceptedOnL2(transition);
                        break Ok(transition);
                    }
                }
                // This happens if the channel lags behind the mempool
                Err(_) => {
                    let block_info = common
                        .starknet
                        .backend
                        .find_tx_hash_block_info(&common.tx_hash)
                        .or_else_internal_server_error(|| {
                            format!("SubscribeTransactionStatus failed to retrieve block info for tx {tx_hash:#x}")
                        })?;

                    let Some((mp_block::MadaraMaybePendingBlockInfo::NotPending(block_info), _idx)) = block_info else {
                        continue;
                    };

                    let block_number = block_info.header.block_number;
                    let transition = StateTransitionAcceptedOnL1 { common, block_number, channel_confirmed };
                    let transition = Self::TransitionTo::WaitAcceptedOnL1(transition);
                    break Ok(transition);
                }
            }
        }
    }
}
impl<'a> StateTransition for StateTransitionAcceptedOnL2<'a> {
    type TransitionTo = StateTransitionAcceptedOnL1<'a>;

    async fn transition(self) -> Result<Self::TransitionTo, crate::errors::StarknetWsApiError> {
        let Self { common, mut channel_pending_tx } = self;

        let channel_confirmed = common.starknet.backend.subscribe_last_block_on_l1();
        let tx_hash = &common.tx_hash;

        // Step 1: we wait for the tx to be ACCEPTED in the pending block
        while let Ok(tx) = channel_pending_tx.recv().await {
            if tx.receipt.transaction_hash() == common.tx_hash {
                break;
            }
        }

        // We don't care if this skips an update, we only use this to check against the db on every
        // pending block tick
        let mut channel_pending_block = common.starknet.backend.subscribe_pending_block();

        // Step 2: we wait for the tx to be INCLUDED into the pending block. This is necessary since
        // the pending tx channel is actually a bit in advance compared to block production and will
        // broadcast txs in their batch phase before they are included in the pending block.
        let block_number = loop {
            let block_info =
                common.starknet.backend.find_tx_hash_block_info(&common.tx_hash).or_else_internal_server_error(
                    || format!("SubscribeTransactionStatus failed to retrieve block info for tx {tx_hash:#x}"),
                )?;
            match block_info {
                Some((mp_block::MadaraMaybePendingBlockInfo::Pending(block_info), _idx)) => {
                    break block_info.header.parent_block_number.map(|n| n + 1).unwrap_or(0);
                }
                Some((mp_block::MadaraMaybePendingBlockInfo::NotPending(block_info), _idx)) => {
                    break block_info.header.block_number;
                }
                None => channel_pending_block
                    .changed()
                    .await
                    .or_internal_server_error("SubscribeTransactionStatus failed to wait for watch channel update")?,
            }
        };

        Ok(Self::TransitionTo { common, block_number, channel_confirmed })
    }
}
impl<'a> StateTransition for StateTransitionAcceptedOnL1<'a> {
    type TransitionTo = StateTransitionEnd<'a>;

    async fn transition(self) -> Result<Self::TransitionTo, crate::errors::StarknetWsApiError> {
        let Self { common, block_number, mut channel_confirmed } = self;

        loop {
            let confirmed = channel_confirmed.borrow_and_update().to_owned();
            if confirmed.is_some_and(|n| block_number <= n) {
                break Ok(Self::TransitionTo { common });
            }

            // **FOOTGUN!** 💥
            //
            // We only wait for L1 confirmed updates AFTER an initial check. This is because all
            // previously sent values in a `tokio::sync::watch` channel are marked as seen when we
            // first subscribe. If the subscription happens right after an L1 state update, that
            // means we would have to wait yet another update before we could read its state, and
            // since those are quite infrequent, that can be a lot of time!
            channel_confirmed
                .changed()
                .await
                .or_internal_server_error("SubscribeTransactionStatus failed to wait for watch channel update")?;
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        versions::user::v0_8_0::{StarknetWsRpcApiV0_8_0Client, StarknetWsRpcApiV0_8_0Server},
        Starknet,
    };

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const TX_HASH: starknet_types_core::felt::Felt = starknet_types_core::felt::Felt::from_hex_unchecked(
        "0x3ccaabf599097d1965e1ef8317b830e76eb681016722c9364ed6e59f3252908",
    );

    #[rstest::fixture]
    fn logs() {
        let debug = tracing_subscriber::filter::LevelFilter::DEBUG;
        let env = tracing_subscriber::EnvFilter::builder().with_default_directive(debug.into()).from_env_lossy();
        let timer = tracing_subscriber::fmt::time::Uptime::default();
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter(env)
            .with_file(true)
            .with_line_number(true)
            .with_target(false)
            .with_timer(timer)
            .try_init();
    }

    #[rstest::fixture]
    fn tx() -> mp_rpc::BroadcastedInvokeTxn {
        mp_rpc::BroadcastedInvokeTxn::V0(mp_rpc::InvokeTxnV0 {
            calldata: Default::default(),
            contract_address: Default::default(),
            entry_point_selector: Default::default(),
            max_fee: Default::default(),
            signature: Default::default(),
        })
    }

    #[rstest::fixture]
    fn tx_with_receipt(tx: mp_rpc::BroadcastedInvokeTxn) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(tx.into()),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash: TX_HASH,
                ..Default::default()
            }),
        }
    }

    #[rstest::fixture]
    fn pending(tx_with_receipt: mp_block::TransactionWithReceipt) -> mp_block::PendingFullBlock {
        mp_block::PendingFullBlock {
            header: Default::default(),
            state_diff: Default::default(),
            transactions: vec![tx_with_receipt],
            events: Default::default(),
        }
    }

    #[rstest::fixture]
    fn block(tx_with_receipt: mp_block::TransactionWithReceipt) -> mp_block::MadaraMaybePendingBlock {
        mp_block::MadaraMaybePendingBlock {
            info: mp_block::MadaraMaybePendingBlockInfo::NotPending(mp_block::MadaraBlockInfo {
                tx_hashes: vec![TX_HASH],
                ..Default::default()
            }),
            inner: mp_block::MadaraBlockInner {
                transactions: vec![tx_with_receipt.transaction],
                receipts: vec![tx_with_receipt.receipt],
            },
        }
    }

    #[rstest::fixture]
    fn starknet() -> Starknet {
        let chain_config = std::sync::Arc::new(mp_chain_config::ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let validation = mc_submit_tx::TransactionValidatorConfig { disable_validation: true, disable_fee: false };
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

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_received_before(
        _logs: (),
        starknet: Starknet,
        tx: mp_rpc::BroadcastedInvokeTxn,
    ) {
        let provider = std::sync::Arc::clone(&starknet.add_transaction_provider);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        provider.submit_invoke_transaction(tx).await.expect("Failed to submit invoke transaction");
        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::Received
                });
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_received_after(
        _logs: (),
        starknet: Starknet,
        tx: mp_rpc::BroadcastedInvokeTxn,
    ) {
        let provider = std::sync::Arc::clone(&starknet.add_transaction_provider);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");
        provider.submit_invoke_transaction(tx).await.expect("Failed to submit invoke transaction");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::Received
                });
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_accepted_on_l2_before(
        _logs: (),
        starknet: Starknet,
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
        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
                });
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_accepted_on_l2_after(
        _logs: (),
        starknet: Starknet,
        tx: mp_rpc::BroadcastedInvokeTxn,
        tx_with_receipt: mp_block::TransactionWithReceipt,
        pending: mp_block::PendingFullBlock,
    ) {
        let provider = std::sync::Arc::clone(&starknet.add_transaction_provider);
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        provider.submit_invoke_transaction(tx).await.expect("Failed to submit invoke transaction");
        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::Received
                });
            }
        );

        backend.on_new_pending_tx(tx_with_receipt);
        backend.store_pending_block(pending).expect("Failed to store pending block");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
                });
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_accepted_on_l1_before(
        _logs: (),
        starknet: Starknet,
        block: mp_block::MadaraMaybePendingBlock,
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

        let state_diff = Default::default();
        let converted_classes = Default::default();
        backend.store_block(block, state_diff, converted_classes).expect("Failed to store block");
        backend.write_last_confirmed_block(0).expect("Failed to update last confirmed block");
        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1
                });
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_accepted_on_l1_after(
        _logs: (),
        starknet: Starknet,
        block: mp_block::MadaraMaybePendingBlock,
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

        let state_diff = Default::default();
        let converted_classes = Default::default();
        backend.store_block(block, state_diff, converted_classes).expect("Failed to store block");
        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
                });
            }
        );

        // We assume on our state machine that it is not possible to go from Received directly to
        // AcceptedOnL1. This introduces an extra intermediate state to the status check where we
        // wait for AcceptedOnL2, which is why this test takes place in two phases.
        backend.write_last_confirmed_block(0).expect("Failed to update last confirmed block");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1
                });
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_transaction_status_full_flow(
        _logs: (),
        starknet: Starknet,
        tx: mp_rpc::BroadcastedInvokeTxn,
        tx_with_receipt: mp_block::TransactionWithReceipt,
        pending: mp_block::PendingFullBlock,
        block: mp_block::MadaraMaybePendingBlock,
    ) {
        let provider = std::sync::Arc::clone(&starknet.add_transaction_provider);
        let backend = std::sync::Arc::clone(&starknet.backend);

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        provider.submit_invoke_transaction(tx).await.expect("Failed to submit invoke transaction");
        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::Received
                });
            }
        );

        tracing::debug!("Received");

        backend.on_new_pending_tx(tx_with_receipt);
        backend.store_pending_block(pending).expect("Failed to store pending block");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
                });
            }
        );

        tracing::debug!("AcceptedOnL2");

        let state_diff = Default::default();
        let converted_classes = Default::default();
        backend.store_block(block, state_diff, converted_classes).expect("Failed to store block");
        backend.write_last_confirmed_block(0).expect("Failed to update last confirmed block");

        assert_matches::assert_matches!(
            sub.next().await, Some(Ok(status)) => {
                assert_eq!(status, mp_rpc::v0_8_1::TxnStatus {
                    transaction_hash: TX_HASH,
                    status: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1
                });
            }
        );

        tracing::debug!("AcceptedOnL1");
    }
}
