use crate::{
    rocksdb::RocksDBStorage, storage::{MadaraStorage, MadaraStorageRead, TxIndex}, topic_pubsub::{TopicWatchPubsub, TopicWatchReceiver}, view::{Anchor, PreconfirmedBlockView}, MadaraBackend
};
use anyhow::Context;
use dashmap::DashMap;
use mp_convert::Felt;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionStatus {
    Preconfirmed(PreConfirmationStatus),
    Confirmed { block_n: u64, transaction_index: u64, on_l1: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreConfirmationStatus {
    Received,
    Candidate { view: PreconfirmedBlockView, transaction_index: u64 },
    Preconfirmed { view: PreconfirmedBlockView, transaction_index: u64 },
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(transparent)]
pub struct BlockNStatus(AtomicU64);

impl fmt::Debug for BlockNStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.current())
    }
}

impl BlockNStatus {
    pub fn current(&self) -> Option<u64> {
        self.next().checked_sub(1)
    }
    pub fn next(&self) -> u64 {
        self.0.load(Relaxed)
    }
    pub fn set_current(&self, block_n: Option<u64>) {
        self.0.store(block_n.map(|block_n| block_n + 1).unwrap_or(0), Relaxed)
    }
}

impl Clone for BlockNStatus {
    fn clone(&self) -> Self {
        Self(self.0.load(Relaxed).into())
    }
}

#[derive(Debug)]
pub struct TransactionStatusManager {
    transaction_status_pubsub: TopicWatchPubsub<Felt, TransactionStatus>,
    preconfirmed_transactions: DashMap<Felt, PreConfirmationStatus>,

    l1_head: BlockNStatus,
}

/// Subscription to transaction statuses.
/// This struct only notifies of the most recent known status when it changes. The current value
/// can be returned using [`TransactionStatusSubscription::current`]. This current value will not change
/// until you either call [`TransactionStatusSubscription::refresh`], or you listen to the subscription using
/// [`TransactionStatusSubscription::recv`] and an update is processed.
///
/// Note that it is possible to listen to a transaction status for a transaction that has not arrived yet. In that
/// case, [`TransactionStatusSubscription::current`] will return [`None`] at first.
/// In addition, if, for example, a transaction is removed from the mempool (transition from `Some(Received)` -> `None`), but
/// re-added later (transition from `None` -> `Some(Received)`), the new status will be seen by this subscription and you do
/// not need to resubscribe.
///
/// # Lag behavior
///
/// When too many events happen and the receiver takes too long to handle them, this receiver will skip some events
/// and only return the most recent one. The last event for any transaction may either be a notification that it has
/// been confirmed, or the [`None`] stauts which means it was dropped from the mempool and all pre-confirmed state.
pub struct TransactionStatusSubscription {
    current_value: Option<TransactionStatus>,
    subscription: TopicWatchReceiver<Felt, Option<TransactionStatus>>,
}

impl TransactionStatusSubscription {
    fn new(subscription: TopicWatchReceiver<Felt, Option<TransactionStatus>>) -> Self {
        Self { current_value: subscription.current().clone(), subscription }
    }
    pub fn current(&self) -> &Option<TransactionStatus> {
        &self.current_value
    }
    pub fn refresh(&mut self) {
        self.current_value = self.subscription.borrow_and_update().clone();
    }
    pub async fn recv(&self) -> &Option<TransactionStatus> {
        self.current_value = self.subscription.recv().await;
        &self.current_value
    }
}

impl<DB: MadaraStorage> MadaraBackend<DB> {
    pub fn get_transaction_status(&self, transaction_hash: Felt) -> anyhow::Result<Option<TransactionStatus>> {
        if let Some(got) = self.transaction_statuses.preconfirmed_transactions.get(&transaction_hash) {
            return Some(TransactionStatus::Preconfirmed(got.clone()));
        }
        Ok(self.db.find_transaction_hash(&transaction_hash)?.map(|TxIndex { block_n, transaction_index }| {
            TransactionStatus::Confirmed {
                block_n,
                transaction_index,
                on_l1: self.transaction_statuses.l1_head.current() >= block_n,
            }
        }))
    }

    /// Subscribe to transaction statuses. See [`TransactionStatusSubscription`] for more details.
    pub fn subscribe_transaction_status(
        &self,
        transaction_hash: Felt,
    ) -> anyhow::Result<TransactionStatusSubscription> {
        TransactionStatusSubscription::new(
            self.transaction_statuses
                .transaction_status_pubsub
                .subscribe(transaction_hash, || self.get_transaction_status(&transaction_hash)),
        )
    }
}
