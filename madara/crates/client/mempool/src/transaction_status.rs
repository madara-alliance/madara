use crate::{topic_pubsub::TopicWatchReceiver, Mempool};
use mc_db::{
    preconfirmed::PreconfirmedBlock, view::ExecutedTransactionWithBlockView, MadaraBackend, MadaraStorageRead,
};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionStatus {
    Preconfirmed(PreConfirmationStatus),
    Confirmed { block_number: u64, transaction_index: u64, is_on_l1: bool },
}

impl TransactionStatus {
    pub fn as_preconfirmed(&self) -> Option<&PreConfirmationStatus> {
        match self {
            Self::Preconfirmed(status) => Some(status),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreConfirmationStatus {
    Received(Arc<ValidatedTransaction>),
    Candidate { view: Arc<PreconfirmedBlock>, transaction_index: u64, transaction: Arc<ValidatedTransaction> },
    Executed { view: Arc<PreconfirmedBlock>, transaction_index: u64 },
}

/// Subscription to transaction statuses.
/// This struct only notifies of the most recent known status when it changes. The current value
/// can be returned using [`WatchTransactionStatus::current`]. This current value will not change
/// until you either call [`WatchTransactionStatus::refresh`], or you listen to the subscription using
/// [`WatchTransactionStatus::recv`] and an update is processed.
///
/// Note that it is possible to listen to a transaction status for a transaction that has not arrived yet. In that
/// case, [`WatchTransactionStatus::current`] will return [`None`] at first.
/// In addition, if, for example, a transaction is removed from the mempool (transition from `Some(Received)` -> `None`), but
/// re-added later (transition from `None` -> `Some(Received)`), the new status will be seen by this subscription and you do
/// not need to resubscribe.
///
/// # Lag behavior
///
/// When too many events happen and the receiver takes too long to handle them, this receiver will skip some events
/// and only return the most recent one. The last event for any transaction may either be a notification that it has
/// been confirmed, or the [`None`] stauts which means it was dropped from the mempool and all pre-confirmed state.
pub struct WatchTransactionStatus<D: MadaraStorageRead> {
    _backend: Arc<MadaraBackend<D>>,
    current_value: Option<TransactionStatus>,
    subscription: TopicWatchReceiver<Felt, Option<TransactionStatus>>,
}

impl<D: MadaraStorageRead> WatchTransactionStatus<D> {
    fn new(backend: &Arc<MadaraBackend<D>>, subscription: TopicWatchReceiver<Felt, Option<TransactionStatus>>) -> Self {
        let current_value = subscription.borrow().clone();
        Self { _backend: backend.clone(), current_value, subscription }
    }
    pub fn current(&self) -> &Option<TransactionStatus> {
        &self.current_value
    }
    pub fn refresh(&mut self) {
        self.current_value = self.subscription.borrow_and_update().clone();
    }
    pub async fn recv(&mut self) -> &Option<TransactionStatus> {
        self.subscription.changed().await;
        self.current_value = self.subscription.borrow_and_update().clone();
        &self.current_value
    }
}

impl<D: MadaraStorageRead> Mempool<D> {
    pub fn get_transaction_status(&self, transaction_hash: &Felt) -> anyhow::Result<Option<TransactionStatus>> {
        if let Some(got) = self.preconfirmed_transactions_statuses.get(transaction_hash) {
            return Ok(Some(TransactionStatus::Preconfirmed(got.clone())));
        }
        let view = self.backend.view_on_latest();
        Ok(view.find_transaction_by_hash(transaction_hash)?.map(
            |ExecutedTransactionWithBlockView { transaction_index, block }| TransactionStatus::Confirmed {
                block_number: block.block_number(),
                transaction_index,
                is_on_l1: self.backend.latest_l1_confirmed_block_n().is_some_and(|l1| l1 >= block.block_number()),
            },
        ))
    }

    pub fn is_transaction_in_mempool(&self, transaction_hash: &Felt) -> bool {
        self.preconfirmed_transactions_statuses
            .get(transaction_hash)
            .is_some_and(|h| matches!(h.value(), &PreConfirmationStatus::Received(_)))
    }

    /// Subscribe to transaction statuses. See [`WatchTransactionStatus`] for more details.
    pub fn watch_transaction_status(
        self: &Arc<Self>,
        transaction_hash: Felt,
    ) -> anyhow::Result<WatchTransactionStatus<D>> {
        Ok(WatchTransactionStatus::new(
            &self.backend,
            self.watch_transaction_status.watch(transaction_hash, || self.get_transaction_status(&transaction_hash))?,
        ))
    }
}
