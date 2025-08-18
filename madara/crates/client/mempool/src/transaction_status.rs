use crate::{
    prelude::*,
    storage::{MadaraStorageRead, TxIndex},
    topic_pubsub::{TopicWatchPubsub, TopicWatchReceiver},
    view::{MadaraBlockView, PreconfirmedBlockAnchor},
    MadaraBackend,
};
use dashmap::DashMap;
use futures::future::OptionFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionStatus {
    Preconfirmed(PreConfirmationStatus),
    Confirmed { block_n: u64, transaction_index: u64, on_l1: bool },
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
    Received,
    Candidate { view: PreconfirmedBlockAnchor, transaction_index: u64 },
    Executed { view: PreconfirmedBlockAnchor, transaction_index: u64 },
}

pub struct TransactionStatusManager<D> {
    backend: Arc<MadaraBackend<D>>,
    /// Pubsub for transaction statuses.
    watch_transaction_status: TopicWatchPubsub<Felt, Option<TransactionStatus>>,
    /// All current transaction statuses for mempool & preconfirmed block.
    preconfirmed_transactions_statuses: DashMap<Felt, PreConfirmationStatus>,
}

impl<D: MadaraStorageRead> TransactionStatusManager<D> {
    pub fn new(backend: Arc<MadaraBackend<D>>) -> Self {
        Self {
            backend,
            watch_transaction_status: Default::default(),
            preconfirmed_transactions_statuses: Default::default(),
        }
    }
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
        self.subscription.changed();
        self.current_value = self.subscription.borrow_and_update().clone();
        &self.current_value
    }
}

impl<D: MadaraStorageRead> TransactionStatusManager<D> {
    fn set_transaction_status(&self, tx_hash: &Felt, value: Option<TransactionStatus>) {
        if let Some(value) = value.as_ref().and_then(|status| status.as_preconfirmed()).clone() {
            self.preconfirmed_transactions_statuses.insert(*tx_hash, value.clone());
        }
        self.watch_transaction_status.publish(tx_hash, value);
    }
    fn set_transaction_status_in_block(&self, view: &MadaraBlockView<D>, tx_hash: &Felt, transaction_index: u64) {
        if let Some(preconfirmed) = view.anchor.as_preconfirmed() {
            if transaction_index >= preconfirmed.n_executed() as u64 {
                self.set_transaction_status(
                    tx_hash,
                    Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Candidate {
                        view: preconfirmed.clone(),
                        transaction_index,
                    })),
                )
            } else {
                self.set_transaction_status(
                    tx_hash,
                    Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Executed {
                        view: preconfirmed.clone(),
                        transaction_index,
                    })),
                )
            }
        } else {
            self.set_transaction_status(
                tx_hash,
                Some(TransactionStatus::Confirmed {
                    block_n: view.block_n(),
                    transaction_index,
                    on_l1: view.is_on_l1(),
                }),
            )
        }
    }

    pub fn get_transaction_status(&self, transaction_hash: &Felt) -> anyhow::Result<Option<TransactionStatus>> {
        if let Some(got) = self.preconfirmed_transactions_statuses.get(&transaction_hash) {
            return Ok(Some(TransactionStatus::Preconfirmed(got.clone())));
        }
        Ok(self.backend.db.find_transaction_hash(&transaction_hash)?.map(|TxIndex { block_n, transaction_index }| {
            TransactionStatus::Confirmed {
                block_n,
                transaction_index,
                on_l1: self.backend.latest_l1_confirmed_block_n().is_some_and(|l1| l1 >= block_n),
            }
        }))
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

    async fn run_update_statuses_task(self: Arc<Self>) -> Result<()> {
        let mut l1_new_heads_subscription = self.backend.subscribe_new_l1_confirmed_heads();

        let mut new_heads_subscription =
            self.backend.subscribe_new_heads(crate::subscription::SubscribeNewBlocksTag::Preconfirmed);
        // Start returning heads from the next block after the latest confirmed block (inclusive).
        new_heads_subscription
            .set_start_from(self.backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(/* genesis */ 0));

        let mut current_head = new_heads_subscription.current_block_view();

        loop {
            let mut removed: HashMap<Felt, mp_block::Transaction> = HashMap::new();
            tokio::select! {
                biased;

                // Process updates confirmed on l1 first
                new_head_on_l1 = l1_new_heads_subscription.next_block_view() => {
                    for (tx_index, tx) in new_head_on_l1.get_block_transactions(..)?.iter().enumerate() {
                        self.set_transaction_status_in_block(
                            &new_head_on_l1,
                            tx.receipt.transaction_hash(),
                            tx_index as u64,
                        );
                    }

                    // If the l2 subscription is behind the l1 subscription, we want to skip ahead because we already handled these
                    // blocks.
                    if l1_new_heads_subscription.current() > &new_heads_subscription.current().latest_confirmed_block_n() {
                        new_heads_subscription.set_start_from(l1_new_heads_subscription.current().map(|n| n + 1).unwrap_or(0));

                        // If the previous head was preconfirmed, mark all of its transactions as removed.
                        // remove_preconfirmed_txs(current_head)?;
                        let new_head = new_head_on_l1;
                        if let Some(preconfirmed) = current_head.as_ref().and_then(|v| v.to_preconfirmed()) {
                            for tx in preconfirmed.get_block_transactions_with_candidates(..) {
                                removed.insert(*tx.hash(), tx.transaction().clone());
                            }
                        }
                        current_head = Some(new_head);
                    }
                }

                // New block on l2
                new_head = new_heads_subscription.next_block_view() => {
                    // Reset preconfirmed block state in every case.

                    if let Some(preconfirmed) = current_head.as_ref().and_then(|v| v.to_preconfirmed()) {
                        for tx in preconfirmed.get_block_transactions_with_candidates(..) {
                            removed.insert(*tx.hash(), tx.transaction().clone());
                        }
                    }

                    if let Some(preconfirmed) = current_head.as_ref().and_then(|v| v.to_preconfirmed()) {
                        for (tx_index, tx) in preconfirmed.get_block_transactions_with_candidates(..).iter().enumerate() {
                            removed.remove(tx.hash());
                            self.set_transaction_status_in_block(
                                &new_head,
                                tx.hash(),
                                tx_index as u64,
                            );
                        }
                    } else {
                        for (tx_index, tx) in new_head.get_block_transactions(..)?.iter().enumerate() {
                            removed.remove(tx.receipt.transaction_hash());
                            self.set_transaction_status_in_block(
                                &new_head,
                                tx.receipt.transaction_hash(),
                                tx_index as u64,
                            );
                        }
                    }
                    current_head = Some(new_head);

                }

                // Preconfirmed block new tx.
                Some(view) = OptionFuture::from(current_head.as_mut().and_then(|v| v.to_preconfirmed()).map(|mut v| async {
                    v.wait_until_outdated().await;
                    v
                })) => {
                    let (old_view, mut new_head) = (view.clone(), view);
                    new_head.refresh_with_candidates();

                    for el in old_view.get_candidates() {
                        removed.insert(el.hash, el.transaction.clone());
                    }

                    let new_head = new_head.into_block_view();
                    for (tx_index, tx) in new_head.get_block_transactions(old_view.n_executed() as u64..)?.iter().enumerate() {
                        removed.remove(tx.receipt.transaction_hash());
                        self.set_transaction_status_in_block(
                            &new_head,
                            tx.receipt.transaction_hash(),
                            tx_index as u64,
                        );
                    }
                    current_head = Some(new_head)
                }
            }

            for (tx_hash, tx) in removed {
                // add back to mempool
                self.set_transaction_status(&tx_hash, Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Received)));
            }
        }
    }
}
