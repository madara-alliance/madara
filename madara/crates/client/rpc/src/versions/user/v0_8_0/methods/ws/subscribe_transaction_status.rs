use crate::errors::ErrorExtWs;

/// Notifies the subscriber of updates to a transaction's status. ([specs])
///
/// Supported statuses are:
///
/// - **Received**: tx has been inserted into the mempool.
/// - **Rejected**: tx was included into a block but failed execution.
/// - **Accepted on L2**: tx has been inserted into the pending block.
/// - **Accepted on L1**: tx has been finalized on L1.
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

    // TODO: timeout should be based off a constant in chain config
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(300), state.drive());

    tokio::select! {
        res = timeout => res.or_else_internal_server_error(|| {
            format!("SubscribeTransactionStatus timed out on {transaction_hash:#x}")
        })?,
        _ = sink.closed() => Ok(())
    }
}

#[derive(Default)]
enum SubscriptionState<'a> {
    #[default]
    None,
    WaitReceived(StateTransitionReceived<'a>),
    WaitAcceptedOnL2(StateTransitionAcceptedOnL2<'a>),
    WaitAcceptedOnL1(StateTransitionAcceptedOnL1<'a>),
}

impl<'a> SubscriptionState<'a> {
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
                mp_block::MadaraMaybePendingBlockInfo::Pending(_) => {
                    common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                    Ok(Self::WaitAcceptedOnL1(StateTransitionAcceptedOnL1 { common, channel: todo!() }))
                }
                mp_block::MadaraMaybePendingBlockInfo::NotPending(_) => {
                    common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1).await?;
                    Ok(Self::None)
                }
            }
        } else {
            Ok(Self::WaitReceived(StateTransitionReceived { common, channel: todo!() }))
        }
    }

    /// ```text
    ///          ┌────┐
    ///        ┌►│None├─────────────────────────────────────────────►──┐
    ///        │ └────┘                                                │
    /// ┌─────┐│ ┌────────────┐  ┌────────────────┐  ┌────────────────┐│ ┌───┐
    /// │START├┴►│WaitReceived├─►│WaitAcceptedOnL2├┬►│WaitAcceptedOnL1├┼►│END│
    /// └─────┘  └────────────┘  └────────────────┘│ └────────────────┘│ └───┘
    ///                                            │ ┌────────┐        │
    ///                                            └►│Rejected├─────►──┘
    ///                                              └────────┘
    ///
    /// ```
    async fn drive(&mut self) -> Result<(), crate::errors::StarknetWsApiError> {
        loop {
            match std::mem::take(self) {
                Self::None => return Ok(()),
                Self::WaitReceived(state) => match state.transition().await? {
                    StateTransitionResult::State(s) => *self = Self::WaitReceived(s),
                    StateTransitionResult::Transition(s) => {
                        s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Received).await?;
                        *self = Self::WaitAcceptedOnL2(s);
                    }
                },
                Self::WaitAcceptedOnL2(state) => match state.transition().await? {
                    StateTransitionResult::State(s) => *self = Self::WaitAcceptedOnL2(s),
                    StateTransitionResult::Transition(s) => match s {
                        StateMatrixAcceptedOnL2::Rejected(s) => {
                            s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Rejected).await?;
                            return Ok(());
                        }
                        StateMatrixAcceptedOnL2::WaitAcceptedOnL1(s) => {
                            s.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                            *self = Self::WaitAcceptedOnL1(s);
                        }
                    },
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
struct StateTransitionRejected<'a> {
    common: StateTransitionCommon<'a>,
}
struct StateTransitionAcceptedOnL2<'a> {
    common: StateTransitionCommon<'a>,
    channel: tokio::sync::watch::Receiver<std::sync::Arc<mp_block::MadaraPendingBlockInfo>>,
}
struct StateTransitionAcceptedOnL1<'a> {
    common: StateTransitionCommon<'a>,
    channel: tokio::sync::watch::Receiver<mp_convert::Felt>,
}

enum StateMatrixAcceptedOnL2<'a> {
    WaitAcceptedOnL1(StateTransitionAcceptedOnL1<'a>),
    Rejected(StateTransitionRejected<'a>),
}

impl<'a> StateTransitionCommon<'a> {
    fn new(
        starknet: &'a crate::Starknet,
        subscription_sink: &'a jsonrpsee::core::server::SubscriptionSink,
        transaction_hash: mp_convert::Felt,
    ) -> Self {
        Self { starknet, subscription_sink, transaction_hash }
    }

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
    type TransitionTo = StateTransitionAcceptedOnL2<'a>;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        let Self { common, channel } = self;
        Ok(StateTransitionResult::Transition(Self::TransitionTo {
            channel: common.starknet.backend.subscribe_pending_block(),
            common,
        }))
    }
}
impl<'a> StateTransition for StateTransitionAcceptedOnL2<'a> {
    type TransitionTo = StateMatrixAcceptedOnL2<'a>;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        let Self { common, mut channel } = self;

        let block_info = std::sync::Arc::clone(&channel.borrow_and_update());
        if block_info.tx_hashes.iter().find(|hash| *hash == &common.transaction_hash).is_some() {
            let channel = todo!();
            let transition = StateTransitionAcceptedOnL1 { common, channel };
            Ok(StateTransitionResult::Transition(Self::TransitionTo::WaitAcceptedOnL1(transition)))
        } else {
            let block_info = common
                .starknet
                .backend
                .find_tx_hash_block_info(&common.transaction_hash)
                .or_else_internal_server_error(|| {
                    format!("Error looking for block info associated to tx {:#x}", common.transaction_hash)
                })?;

            if block_info.is_some() {
                let channel = todo!();
                let transition = Self::TransitionTo::WaitAcceptedOnL1(StateTransitionAcceptedOnL1 { common, channel });
                Ok(StateTransitionResult::Transition(transition))
            } else {
                Ok(StateTransitionResult::State(Self { common, channel }))
            }
        }
    }
}
impl<'a> StateTransition for StateTransitionAcceptedOnL1<'a> {
    type TransitionTo = StateTransitionAcceptedOnL1<'a>;

    async fn transition(
        self,
    ) -> Result<StateTransitionResult<Self, Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        todo!()
    }
}
