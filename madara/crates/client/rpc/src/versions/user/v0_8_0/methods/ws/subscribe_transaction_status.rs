use crate::errors::ErrorExtWs;

/// Notifies the subscriber of updates to a transaction's status. ([specs])
///
/// Supported statuses are:
///
/// - **Received**: tx has been inserted into the mempool.
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
    let mut state = SubscriptionState::new(starknet, &sink, transaction_hash);

    // TODO: timeout should be based off a constant in chain config
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(300), state.drive());

    tokio::select! {
        res = timeout => res.or_internal_server_error("Failed to timeout on transaction status")?,
        _ = sink.closed() => Ok(())
    }
}

enum SubscriptionState<'a> {
    Received(StateTransitionReceived<'a>),
    AcceptedOnL2(StateTransitionAcceptedOnL2<'a>),
    AcceptedOnL1(StateTransitionAcceptedOnL1<'a>),
}

impl<'a> SubscriptionState<'a> {
    fn new(
        starknet: &'a crate::Starknet,
        subscription_sink: &'a jsonrpsee::core::server::SubscriptionSink,
        transaction_hash: mp_convert::Felt,
    ) -> Self {
        todo!()
    }

    async fn drive(&mut self) -> Result<(), crate::errors::StarknetWsApiError> {
        loop {
            match self {
                SubscriptionState::Received(state) => {
                    if let Some(state) = state.transition().await? {
                        state.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::Received).await?;
                        *self = Self::AcceptedOnL2(state);
                    }
                }
                SubscriptionState::AcceptedOnL2(state) => {
                    if let Some(state) = state.transition().await? {
                        state.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2).await?;
                        *self = Self::AcceptedOnL1(state);
                    }
                }
                SubscriptionState::AcceptedOnL1(state) => {
                    if let Some(state) = state.transition().await? {
                        state.common.send_txn_status(mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1).await?;
                        return Ok(());
                    }
                }
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
    channel: tokio::sync::watch::Receiver<mp_convert::Felt>,
}
struct StateTransitionAcceptedOnL1<'a> {
    common: StateTransitionCommon<'a>,
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

trait StateTransition {
    type TransitionTo: StateTransition;

    async fn transition(&mut self) -> Result<Option<Self::TransitionTo>, crate::errors::StarknetWsApiError>;
}

impl<'a> StateTransition for StateTransitionReceived<'a> {
    type TransitionTo = StateTransitionAcceptedOnL2<'a>;

    async fn transition(&mut self) -> Result<Option<Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        todo!()
    }
}

impl<'a> StateTransition for StateTransitionAcceptedOnL2<'a> {
    type TransitionTo = StateTransitionAcceptedOnL1<'a>;

    async fn transition(&mut self) -> Result<Option<Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        todo!()
    }
}

impl<'a> StateTransition for StateTransitionAcceptedOnL1<'a> {
    type TransitionTo = StateTransitionAcceptedOnL1<'a>;

    async fn transition(&mut self) -> Result<Option<Self::TransitionTo>, crate::errors::StarknetWsApiError> {
        todo!()
    }
}
