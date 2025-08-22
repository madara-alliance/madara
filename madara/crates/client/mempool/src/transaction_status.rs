use crate::{
    topic_pubsub::{TopicWatchPubsub, TopicWatchReceiver},
    Mempool,
};
use anyhow::Context;
use dashmap::DashMap;
use futures::future::OptionFuture;
use mc_db::{
    preconfirmed::PreconfirmedBlock, view::ExecutedTransactionWithBlockView, MadaraBackend, MadaraBlockView,
    MadaraStorageRead,
};
use mp_convert::Felt;
use std::{collections::HashMap, sync::Arc};

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
    Received(Arc<mp_block::Transaction>),
    Candidate { view: Arc<PreconfirmedBlock>, transaction_index: u64, transaction: Arc<mp_block::Transaction> },
    Executed { view: Arc<PreconfirmedBlock>, transaction_index: u64 },
}

pub struct TransactionStatusManager<D> {
    backend: Arc<MadaraBackend<D>>,
    /// Pubsub for transaction statuses.
    watch_transaction_status: TopicWatchPubsub<Felt, Option<TransactionStatus>>,
    /// All current transaction statuses for mempool & preconfirmed block.
    preconfirmed_transactions_statuses: DashMap<Felt, PreConfirmationStatus>,
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

impl<D: MadaraStorageRead> Mempool<D> {
    fn set_transaction_status(&self, tx_hash: Felt, value: Option<TransactionStatus>) {
        if let Some(value) = value.as_ref().and_then(|status| status.as_preconfirmed()).clone() {
            self.preconfirmed_transactions_statuses.insert(tx_hash, value.clone());
        }
        self.watch_transaction_status.publish(&tx_hash, value);
    }

    /// Takes an iterator of items (transaction_hash, transaction_index).
    fn update_block_transaction_statuses(
        &self,
        view: &MadaraBlockView<D>,
        iter: impl IntoIterator<Item = (usize, Felt)>,
        put_back_into_mempool: &mut HashMap<Felt, mp_block::Transaction>,
    ) -> anyhow::Result<()> {
        let is_on_l1 = view.is_on_l1();
        for (tx_index, tx_hash) in iter {
            put_back_into_mempool.remove(&tx_hash); // The transaction should not be re-inserted into mempool.

            if let Some(preconfirmed) = view.as_preconfirmed() {
                if let Some(candidate_index) = usize::checked_sub(tx_index, preconfirmed.num_executed_transactions()) {
                    // transaction_index >= num_executed_transactions, it's a candidate transaction.
                    self.set_transaction_status(
                        tx_hash,
                        Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Candidate {
                            view: preconfirmed.block().clone(),
                            transaction_index: tx_index as u64,
                            transaction: Arc::new(
                                preconfirmed
                                    .candidate_transactions()
                                    .get(candidate_index)
                                    .context("Candidate transaction should be in block")?
                                    .transaction
                                    .clone(),
                            ),
                        })),
                    )
                } else {
                    self.set_transaction_status(
                        tx_hash,
                        Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Executed {
                            view: preconfirmed.block().clone(),
                            transaction_index: tx_index as u64,
                        })),
                    )
                }
            } else {
                self.set_transaction_status(
                    tx_hash,
                    Some(TransactionStatus::Confirmed {
                        block_number: view.block_number(),
                        transaction_index: tx_index as u64,
                        is_on_l1,
                    }),
                )
            }
        }
        Ok(())
    }

    pub fn get_transaction_status(&self, transaction_hash: &Felt) -> anyhow::Result<Option<TransactionStatus>> {
        if let Some(got) = self.preconfirmed_transactions_statuses.get(&transaction_hash) {
            return Ok(Some(TransactionStatus::Preconfirmed(got.clone())));
        }
        let view = self.backend.view_on_latest();
        Ok(view.find_transaction_by_hash(&transaction_hash)?.map(
            |ExecutedTransactionWithBlockView { transaction_index, block }| TransactionStatus::Confirmed {
                block_number: block.block_number(),
                transaction_index,
                is_on_l1: self.backend.latest_l1_confirmed_block_n().is_some_and(|l1| l1 >= block.block_number()),
            },
        ))
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

    /// This task updates all the l1, confirmed, pre-confirmed, candidates statuses by watching the backend chain tip and pre-confirmed block.
    /// It is also responsible for adding transactions back into the mempool.
    async fn run_chain_watcher_task(self: Arc<Self>) -> anyhow::Result<()> {
        let mut l1_new_heads_subscription = self.backend.subscribe_new_l1_confirmed_heads();

        let mut new_heads_subscription =
            self.backend.subscribe_new_heads(mc_db::subscription::SubscribeNewBlocksTag::Preconfirmed);
        // Start returning heads from the next block after the latest confirmed block (inclusive).
        new_heads_subscription
            .set_start_from(self.backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(/* genesis */ 0));

        let mut current_head = new_heads_subscription.current_block_view();

        loop {
            // When the pre-confirmed block changes, we need to put all of the removed transactions back into the mempool.
            // However, we don't want to put them right away: for example, if the pre-confirmed block became confirmed, we don't want to insert
            // the transactions back into the mempool just to remove them right away to mark them confirmed. We use this map to track this.
            let mut put_back_into_mempool: HashMap<Felt, mp_block::Transaction> = HashMap::new();

            // Utility function to add all pre-confirmed transactions to the removed map.
            // This is called everytime the pre-confirmed head changes.
            fn preconfirmed_back_to_mempool<D: MadaraStorageRead>(
                current_head: &Option<MadaraBlockView<D>>,
                put_back_into_mempool: &mut HashMap<Felt, mp_block::Transaction>,
            ) {
                if let Some(preconfirmed) = current_head.as_ref().and_then(|v| v.as_preconfirmed()) {
                    for tx in preconfirmed.get_executed_transactions(..) {
                        put_back_into_mempool.insert(*tx.receipt.transaction_hash(), tx.transaction.clone());
                    }
                    for tx in preconfirmed.candidate_transactions() {
                        put_back_into_mempool.insert(tx.hash, tx.transaction.clone());
                    }
                }
            }

            tokio::select! {
                biased;

                // Preconfirmed block new tx. We process this first to be sure we don't miss transactions.
                Some(preconfirmed) = OptionFuture::from(current_head.as_mut().and_then(|v| v.as_preconfirmed_mut()).map(|v| async {
                    v.wait_until_outdated().await;
                    v
                })) => {
                    // Remove all previous candidates.
                    for tx in preconfirmed.candidate_transactions() {
                        put_back_into_mempool.insert(tx.hash, tx.transaction.clone());
                    }

                    let previous_num_txs = preconfirmed.num_executed_transactions();
                    preconfirmed.refresh_with_candidates();

                    // reborrow as immutable to make the compiler happy :)
                    let current_head = current_head.as_ref().context("Current head should be present")?;
                    let preconfirmed = current_head.as_preconfirmed().context("Current head should be preconfirmed")?;

                    // Process new executed transactions.
                    self.update_block_transaction_statuses(
                        &current_head,
                        preconfirmed.get_block_info().tx_hashes[previous_num_txs..].iter().cloned().enumerate(),
                        &mut put_back_into_mempool,
                    )?;
                    // Candidate transactions.
                    self.update_block_transaction_statuses(
                        &current_head,
                        preconfirmed.candidate_transactions().iter().enumerate().map(|(candidate_index, tx)| {
                            (candidate_index + preconfirmed.num_executed_transactions(), tx.hash)
                        }),
                        &mut put_back_into_mempool,
                    )?;
                }

                // Process updates confirmed on l1 first
                new_head_on_l1 = l1_new_heads_subscription.next_block_view() => {
                    let new_head_on_l1: MadaraBlockView<D> = new_head_on_l1.into();
                    // If the l2 subscription is behind the l1 subscription, we want to skip ahead because we already handled these
                    // blocks.
                    if l1_new_heads_subscription.current() > &new_heads_subscription.current().latest_confirmed_block_n() {
                        new_heads_subscription.set_start_from(l1_new_heads_subscription.current().map(|n| n + 1).unwrap_or(0));

                        // If the previous head was preconfirmed, mark all of its transactions as removed.
                        preconfirmed_back_to_mempool(&current_head, &mut put_back_into_mempool);

                        current_head = Some(new_head_on_l1.clone());
                    }
                    self.update_block_transaction_statuses(
                        &new_head_on_l1,
                        new_head_on_l1.get_block_info()?.tx_hashes().iter().cloned().enumerate(),
                        &mut put_back_into_mempool,
                    )?;
                }

                // New block on l2: either confirmed or pre-confirmed.
                mut new_head = new_heads_subscription.next_block_view() => {
                    // If the previous head was preconfirmed, mark all of its transactions as removed.
                    preconfirmed_back_to_mempool(&current_head, &mut put_back_into_mempool);

                    if let MadaraBlockView::Preconfirmed(preconfirmed) = &mut new_head {
                        preconfirmed.refresh_with_candidates();
                    }

                    // Update statuses for all transactions in new_head.
                    match &new_head {
                        MadaraBlockView::Confirmed(confirmed) => {
                            self.update_block_transaction_statuses(
                                &new_head,
                                confirmed.get_block_info()?.tx_hashes.iter().cloned().enumerate(),
                                &mut put_back_into_mempool,
                            )?;
                        }
                        MadaraBlockView::Preconfirmed(preconfirmed) => {
                            // Executed transactions.
                            self.update_block_transaction_statuses(
                                &new_head,
                                preconfirmed.get_block_info().tx_hashes.iter().cloned().enumerate(),
                                &mut put_back_into_mempool,
                            )?;
                            // Candidate transactions.
                            self.update_block_transaction_statuses(
                                &new_head,
                                preconfirmed.candidate_transactions().iter().enumerate().map(|(candidate_index, tx)| {
                                    (candidate_index + preconfirmed.num_executed_transactions(), tx.hash)
                                }),
                                &mut put_back_into_mempool,
                            )?;
                        }
                    }

                    current_head = Some(new_head);
                }
            }

            for (tx_hash, tx) in put_back_into_mempool {
                // add back to mempool
                self.set_transaction_status(
                    tx_hash,
                    Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Received(Arc::new(tx)))),
                );
            }
        }
    }
}
