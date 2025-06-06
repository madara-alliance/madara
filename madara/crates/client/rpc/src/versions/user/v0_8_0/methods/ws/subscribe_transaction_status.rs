use crate::errors::ErrorExtWs;
use futures::StreamExt;

#[cfg(test)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(test))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300); // 5min

/// Notifies the subscriber of updates to a transaction's status. ([specs])
///
/// Supported statuses are:
///
/// - **Received**: tx has been inserted into the mempool.
/// - **Accepted on L2**: tx has been inserted into the pending block.
/// - **Accepted on L1**: tx has been finalized on L1.
///
/// We do not currently support the **Rejected** transaction status.
///
/// Note that it is possible to call this method on a transaction which has not yet been received by
/// the node and this endpoint will send an update as soon as the tx is received.
///
/// ## DOS mitigation
///
/// To avoid a malicious attacker keeping connections open indefinitely on an nonexistent
/// transaction hash, this endpoint will terminate the connection after a global timeout period.
///
/// This subscription will also automatically close once a transaction has reached `ACCEPTED_ON_L1`.
///
/// [specs]: https://github.com/starkware-libs/starknet-specs/blob/a2d10fc6cbaddbe2d3cf6ace5174dd0a306f4885/api/starknet_ws_api.json#L127C5-L168C7
pub async fn subscribe_transaction_status(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_hash: mp_convert::Felt,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let mut state = SubscriptionState::new(starknet, &sink, transaction_hash).await?;
    let timeout = tokio::time::timeout(TIMEOUT, state.drive());

    tokio::select! {
        // We need to return an error here or jsonrpsee will not terminate the connection for us.
        res = timeout => res.or_else_internal_server_error(|| {
            format!("SubscribeTransactionStatus timed out on {transaction_hash:#x}")
        })?,
        _ = sink.closed() => Ok(())
    }
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
    #[cfg_attr(test, tracing::instrument(skip_all))]
    async fn new(
        starknet: &'a crate::Starknet,
        subscription_sink: &'a jsonrpsee::core::server::SubscriptionSink,
        transaction_hash: mp_convert::Felt,
    ) -> Result<Self, crate::errors::StarknetWsApiError> {
        let common = StateTransitionCommon { starknet, subscription_sink, transaction_hash };
        let block_info =
            starknet.backend.find_tx_hash_block_info(&transaction_hash).or_else_internal_server_error(|| {
                format!("Error looking for block info associated to tx {transaction_hash:#x}")
            })?;

        if let Some((block_info, _idx)) = block_info {
            match block_info {
                // Tx has been accepted in the pending block, hence it is marked as accepted on L2.
                // We wait for it to be accepted on L1
                mp_block::MadaraMaybePendingBlockInfo::Pending(block_info) => {
                    let block_number = block_number_from_pending(common.starknet, &block_info)?;
                    common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                    let channel = common.starknet.backend.subscribe_last_confirmed_block();
                    Ok(Self::WaitAcceptedOnL1(StateTransitionAcceptedOnL1 { common, block_number, channel }))
                }
                mp_block::MadaraMaybePendingBlockInfo::NotPending(block_info) => {
                    let block_number = block_info.header.block_number;
                    let confirmed = common
                        .starknet
                        .backend
                        .get_l1_last_confirmed_block()
                        .or_internal_server_error("Error retrieving last confirmed block")?;

                    // Tx has been accepted on L1, hence it is marked as such. This is the final
                    // stage of the transaction so the state machine is put in its end state.
                    if confirmed.is_some_and(|n| block_number <= n) {
                        common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1).await?;
                        Ok(Self::None)
                    }
                    // Tx has not yet been accepted on L1 but is included on L2, hence it is marked
                    // as accepted on L2. We wait for it to be accepted on L1
                    else {
                        common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                        let channel = common.starknet.backend.subscribe_last_confirmed_block();
                        Ok(Self::WaitAcceptedOnL1(StateTransitionAcceptedOnL1 { common, block_number, channel }))
                    }
                }
            }
        } else {
            // Local mempool is the only AddTransactionProvider which allows us to inspect the state
            // of received transactions. For other providers (such as when forwarding to a remote
            // gateway), we default to assuming that the transaction has been received and wait for
            // it to be accepted on L2.
            match common.starknet.add_transaction_provider.subscribe_new_transactions().await {
                // We wait for the tx to be received
                Some(channel) => Ok(Self::WaitReceived(StateTransitionReceived { common, channel })),
                // We assume the tx has been received and wait for the tx to be accepted on L2
                None => {
                    common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Received).await?;
                    let channel = common.starknet.backend.subscribe_pending_block();
                    Ok(Self::WaitAcceptedOnL2(StateTransitionAcceptedOnL2 { common, channel }))
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
    ///          ┌─►│None├─────────────────────►───────────────────────────────────┐
    ///          │  └────┘                                                         │
    ///          │                                                                 │
    ///          │             ┌──┐                                           ┌──┐ │
    /// ┌─────┐  │  ┌──────────▼─┐│                            ┌──────────────▼─┐│ └─►┌───┐
    /// │START├──┼─►│WaitReceived├┼────────────────────────┬──►│WaitAcceptedOnL1├┴───►│END│
    /// └─────┘  │  └────────────┘│                        │   └────────────────┘     └───┘
    ///          │                │                   ┌──┐ ▲
    ///          │                └───►┌──────────────▼─┐│ │
    ///          └────────────────────►│WaitAcceptedOnL2├┴─┘
    ///                                └────────────────┘
    ///
    /// ```
    #[cfg_attr(test, tracing::instrument())]
    async fn drive(&mut self) -> Result<(), crate::errors::StarknetWsApiError> {
        loop {
            match std::mem::take(self) {
                Self::None => return Ok(()),
                Self::WaitReceived(state) => match state.transition().await? {
                    StateTransitionResult::State(s) => *self = Self::WaitReceived(s),
                    StateTransitionResult::Transition(s) => match s {
                        TransitionMatrixReceived::WaitAcceptedOnL2(s) => {
                            s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Received).await?;
                            *self = Self::WaitAcceptedOnL2(s);
                        }
                        TransitionMatrixReceived::WaitAcceptedOnL1(s) => {
                            s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                            *self = Self::WaitAcceptedOnL1(s);
                        }
                    },
                },
                Self::WaitAcceptedOnL2(state) => match state.transition().await? {
                    StateTransitionResult::State(s) => *self = Self::WaitAcceptedOnL2(s),
                    StateTransitionResult::Transition(s) => {
                        s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                        *self = Self::WaitAcceptedOnL1(s);
                    }
                },
                Self::WaitAcceptedOnL1(state) => match state.transition().await? {
                    StateTransitionResult::State(s) => *self = Self::WaitAcceptedOnL1(s),
                    StateTransitionResult::Transition(s) => {
                        s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1).await?;
                        return Ok(());
                    }
                },
            }
        }
    }
}

struct StateTransitionCommon<'a> {
    starknet: &'a crate::Starknet,
    subscription_sink: &'a jsonrpsee::core::server::SubscriptionSink,
    transaction_hash: mp_convert::Felt,
}
struct StateTransitionReceived<'a> {
    common: StateTransitionCommon<'a>,
    channel: tokio::sync::broadcast::Receiver<mp_convert::Felt>,
}
struct StateTransitionAcceptedOnL2<'a> {
    common: StateTransitionCommon<'a>,
    channel: mc_db::PendingBlockReceiver,
}
struct StateTransitionAcceptedOnL1<'a> {
    common: StateTransitionCommon<'a>,
    block_number: u64,
    channel: mc_db::LastConfirmedBlockReceived,
}
struct StateTransitionEnd<'a> {
    common: StateTransitionCommon<'a>,
}
enum TransitionMatrixReceived<'a> {
    WaitAcceptedOnL2(StateTransitionAcceptedOnL2<'a>),
    WaitAcceptedOnL1(StateTransitionAcceptedOnL1<'a>),
}

impl<'a> StateTransitionCommon<'a> {
    async fn send_txn_status(
        &self,
        status: mp_rpc::v0_7_1::TxnStatus,
    ) -> Result<(), crate::errors::StarknetWsApiError> {
        let txn_status = mp_rpc::v0_8_1::TxnStatus { transaction_hash: self.transaction_hash, status };
        let msg = jsonrpsee::SubscriptionMessage::from_json(&txn_status).or_else_internal_server_error(|| {
            format!("Failed to create response message for status update at tx hash {:#x}", self.transaction_hash)
        })?;

        self.subscription_sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")
    }
}

enum StateTransitionResult<S1: StateTransition, S2> {
    State(S1),
    Transition(S2),
}
trait StateTransition: Sized {
    type TransitionTo;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError>;
}
impl<'a> StateTransition for StateTransitionReceived<'a> {
    type TransitionTo = TransitionMatrixReceived<'a>;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        let Self { common, mut channel } = self;

        let stream = futures::stream::unfold(&mut channel, |channel| async move {
            match channel.recv().await {
                Ok(felt) => Some((felt, channel)),
                Err(_error) => None, // The stream will end if the channel lags or is closed.
            }
        });

        // We only check 100 transactions at a time and if that fails we check against the
        // transaction provider directly and if that fails we check against the db to make sure we
        // have not missed the transaction.
        let received = stream.take(100).any(|hash| async move { hash == common.transaction_hash }).await
            || common
                .starknet
                .add_transaction_provider
                .received_transaction(common.transaction_hash)
                .await
                .unwrap_or(false);

        if received {
            let channel = common.starknet.backend.subscribe_pending_block();
            let transition = StateTransitionAcceptedOnL2 { channel, common };
            let transition = Self::TransitionTo::WaitAcceptedOnL2(transition);
            Ok(StateTransitionResult::Transition(transition))
        } else {
            let block_info = common
                .starknet
                .backend
                .find_tx_hash_block_info(&common.transaction_hash)
                .or_else_internal_server_error(|| {
                    format!("Error looking for block info associated to tx {:#x}", common.transaction_hash)
                })?;

            if let Some((block_info, _idx)) = block_info {
                let block_number = match block_info {
                    mp_block::MadaraMaybePendingBlockInfo::Pending(block_info) => {
                        block_number_from_pending(common.starknet, &block_info)?
                    }
                    mp_block::MadaraMaybePendingBlockInfo::NotPending(block_info) => block_info.header.block_number,
                };

                // We assume that the time to settle on L1 is great enough that we do not bother to
                // check if a transaction went directly from received to accepted on L1.
                let channel = common.starknet.backend.subscribe_last_confirmed_block();
                let transition = StateTransitionAcceptedOnL1 { common, block_number, channel };
                let transition = Self::TransitionTo::WaitAcceptedOnL1(transition);

                Ok(StateTransitionResult::Transition(transition))
            } else {
                Ok(StateTransitionResult::State(Self { common, channel }))
            }
        }
    }
}
impl<'a> StateTransition for StateTransitionAcceptedOnL2<'a> {
    type TransitionTo = StateTransitionAcceptedOnL1<'a>;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        let Self { common, mut channel } = self;

        channel.changed().await.or_internal_server_error("Error waiting for watch channel update")?;

        let block_info = std::sync::Arc::clone(&channel.borrow_and_update());
        if block_info.tx_hashes.iter().find(|hash| *hash == &common.transaction_hash).is_some() {
            let channel = common.starknet.backend.subscribe_last_confirmed_block();
            let block_number = block_number_from_pending(common.starknet, block_info.as_ref())?;
            let transition = Self::TransitionTo { common, block_number, channel };
            Ok(StateTransitionResult::Transition(transition))
        } else {
            let block_info = common
                .starknet
                .backend
                .find_tx_hash_block_info(&common.transaction_hash)
                .or_else_internal_server_error(|| {
                    format!("Error looking for block info associated to tx {:#x}", common.transaction_hash)
                })?;

            match block_info {
                Some((mp_block::MadaraMaybePendingBlockInfo::Pending(block_info), _idx)) => {
                    let channel = common.starknet.backend.subscribe_last_confirmed_block();
                    let block_number = block_number_from_pending(common.starknet, &block_info)?;
                    let transition = Self::TransitionTo { common, block_number, channel };
                    Ok(StateTransitionResult::Transition(transition))
                }
                Some((mp_block::MadaraMaybePendingBlockInfo::NotPending(block_info), _idx)) => {
                    let channel = common.starknet.backend.subscribe_last_confirmed_block();
                    let block_number = block_info.header.block_number;
                    let transition = Self::TransitionTo { common, block_number, channel };
                    Ok(StateTransitionResult::Transition(transition))
                }
                None => Ok(StateTransitionResult::State(Self { common, channel })),
            }
        }
    }
}
impl<'a> StateTransition for StateTransitionAcceptedOnL1<'a> {
    type TransitionTo = StateTransitionEnd<'a>;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        let Self { common, block_number, mut channel } = self;

        channel.changed().await.or_internal_server_error("Error waiting for watch channel update")?;

        let confirmed = channel.borrow_and_update().to_owned();
        if confirmed.is_some_and(|n| block_number <= n) {
            Ok(StateTransitionResult::Transition(Self::TransitionTo { common }))
        } else {
            Ok(StateTransitionResult::State(Self { common, block_number, channel }))
        }
    }
}

fn block_number_from_pending<'a, 'b>(
    starknet: &'a crate::Starknet,
    block_info: &'b mp_block::MadaraPendingBlockInfo,
) -> Result<u64, crate::errors::StarknetWsApiError> {
    Ok(starknet
        .backend
        .get_block_n(&mp_block::BlockId::Hash(block_info.header.parent_block_hash))
        .or_internal_server_error("Failed to get parent block number")?
        .unwrap_or_default()
        .saturating_add(1))
}

#[cfg(test)]
mod test {
    use crate::{
        versions::user::v0_8_0::{StarknetWsRpcApiV0_8_0Client, StarknetWsRpcApiV0_8_0Server},
        Starknet,
    };

    const SERVER_ADDR: &str = "127.0.0.1:0";

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
        let mempool = std::sync::Arc::new(mc_mempool::Mempool::new(
            std::sync::Arc::clone(&backend),
            mc_mempool::MempoolConfig::for_testing(),
        ));
        let mempool_validator = std::sync::Arc::new(mc_submit_tx::TransactionValidator::new(
            mempool,
            std::sync::Arc::clone(&backend),
            mc_submit_tx::TransactionValidatorConfig::default(),
        ));
        let context = mp_utils::service::ServiceContext::new_for_testing();

        Starknet::new(backend, mempool_validator, Default::default(), context)
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(super::TIMEOUT * 10)]
    async fn subscribe_transaction_timeout(_logs: (), starknet: Starknet) {
        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonprsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));

        tracing::debug!(server_url, "Started jsonrpsee server");

        let builder = jsonrpsee::ws_client::WsClientBuilder::default();
        let client = builder.build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        tracing::debug!("Started jsonrpsee client");

        let transaction_hash = starknet_types_core::felt::Felt::ZERO;
        let mut sub = client.subscribe_transaction_status(transaction_hash).await.expect("Failed subscription");

        assert!(sub.next().await.is_none());
    }
}
