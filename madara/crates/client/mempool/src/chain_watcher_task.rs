use crate::{
    transaction_status::{PreConfirmationStatus, TransactionStatus},
    Mempool,
};
use anyhow::Context;
use futures::future::OptionFuture;
use mc_db::{MadaraBlockView, MadaraStorageRead, MadaraStorageWrite};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use mp_utils::service::ServiceContext;
use starknet_api::core::Nonce;
use std::{collections::HashMap, sync::Arc};

impl<D: MadaraStorageRead + MadaraStorageWrite> Mempool<D> {
    fn set_transaction_status(&self, tx_hash: Felt, value: Option<TransactionStatus>) {
        // Update preconfirmed_transactions_statuses:
        // - Remove if value is None (transaction removed) or value is Confirmed (no longer preconfirmed)
        // - Insert/update if value is a preconfirmed status
        match value.as_ref().and_then(|status| status.as_preconfirmed()) {
            Some(preconfirmed_status) => {
                // Insert or update preconfirmed status
                self.preconfirmed_transactions_statuses.insert(tx_hash, preconfirmed_status.clone());
            }
            None => {
                // Transaction is confirmed or removed - clean up from preconfirmed map
                self.preconfirmed_transactions_statuses.remove(&tx_hash);
            }
        }
        self.watch_transaction_status.publish(&tx_hash, value);
    }

    /// Takes an iterator of items (transaction_hash, transaction_index).
    fn update_block_transaction_statuses(
        &self,
        view: &MadaraBlockView<D>,
        iter: impl IntoIterator<Item = (usize, Felt)>,
        removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
    ) -> anyhow::Result<()> {
        let is_on_l1 = view.is_on_l1();
        for (tx_index, tx_hash) in iter {
            removed.remove(&tx_hash); // The transaction was not removed.

            if let Some(preconfirmed) = view.as_preconfirmed() {
                if let Some(candidate_index) = usize::checked_sub(tx_index, preconfirmed.num_executed_transactions()) {
                    // transaction_index >= num_executed_transactions, it's a candidate transaction.
                    self.set_transaction_status(
                        tx_hash,
                        Some(TransactionStatus::Preconfirmed(PreConfirmationStatus::Candidate {
                            view: preconfirmed.block().clone(),
                            transaction_index: tx_index as u64,
                            transaction: preconfirmed
                                .candidate_transactions()
                                .get(candidate_index)
                                .context("Candidate transaction should be in block")?
                                .clone(),
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

    /// This task updates all the l1, confirmed, pre-confirmed, candidates statuses by watching the backend chain tip and pre-confirmed block.
    /// It is also responsible for adding transactions back into the mempool.
    pub(super) async fn run_chain_watcher_task(&self, mut ctx: ServiceContext) -> anyhow::Result<()> {
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
            let mut removed: HashMap<Felt, Arc<ValidatedTransaction>> = HashMap::new();
            let mut put_back_into_mempool = true; // Whether to drop the txs or re-add them into mempool.
            let mut nonce_updates: HashMap<Felt, Felt> = HashMap::new();

            tokio::select! {
                biased;

                // Preconfirmed block new tx. We process this first to make sure we don't miss transactions.
                Some(preconfirmed) = OptionFuture::from(current_head.as_mut().and_then(|v| v.as_preconfirmed_mut()).map(|v| async {
                    v.wait_until_outdated().await;
                    v
                })) => {
                    tracing::debug!("Mempool task: preconfirmed update.");
                    // Candidates that were not executed are most likely rejected transactions. We don't want them back into the mempool.
                    // Otherwise, we run the risk of having these transactions looping from mempool to block building to mempool repeatedly!
                    put_back_into_mempool = false;

                    // Remove all previous candidates.
                    for tx in preconfirmed.candidate_transactions() {
                        removed.insert(tx.hash, tx.clone());
                    }

                    let previous_num_txs = preconfirmed.num_executed_transactions();
                    preconfirmed.refresh_with_candidates();

                    // reborrow as immutable to make the compiler happy :)
                    let current_head = current_head.as_ref().context("Current head should be present")?;
                    let preconfirmed = current_head.as_preconfirmed().context("Current head should be preconfirmed")?;

                    // Process new executed transactions.
                    self.update_block_transaction_statuses(
                        current_head,
                        preconfirmed.get_block_info().tx_hashes[previous_num_txs..].iter().cloned().enumerate(),
                        &mut removed,
                    )?;
                    // Candidate transactions.
                    self.update_block_transaction_statuses(
                        current_head,
                        preconfirmed.candidate_transactions().iter().enumerate().map(|(candidate_index, tx)| {
                            (candidate_index + preconfirmed.num_executed_transactions(), tx.hash)
                        }),
                        &mut removed,
                    )?;

                    // Mark the nonces from the state diff for update.
                    nonce_updates.extend(
                        preconfirmed
                            .borrow_content()
                            .executed_transactions()
                            .skip(previous_num_txs)
                            .flat_map(|tx| tx.state_diff.nonces.iter()),
                    );
                }

                // New block on l2: either confirmed or pre-confirmed.
                mut new_head = new_heads_subscription.next_block_view() => {
                    tracing::debug!("Mempool task: new head.");
                    // If the previous head was preconfirmed, mark all of its transactions as removed.
                    if let Some(preconfirmed) = current_head.as_ref().and_then(|v| v.as_preconfirmed()) {
                        let view_on_parent = preconfirmed.state_view_on_parent();
                        for tx in preconfirmed.borrow_content().executed_transactions() {
                            // re-convert PreconfirmedExecutedTransaction to ValidatedTransaction.
                            removed.insert(*tx.transaction.receipt.transaction_hash(), tx.to_validated().into());
                            // rollback the contract nonce to what it was before the transaction.
                            for key in tx.state_diff.nonces.keys() {
                                nonce_updates.insert(
                                    *key,
                                    // Get from db.
                                    view_on_parent.get_contract_nonce(key)?.unwrap_or(Felt::ZERO),
                                );
                            }
                        }
                        for tx in preconfirmed.candidate_transactions() {
                            removed.insert(tx.hash, tx.clone());
                        }
                    }

                    if let MadaraBlockView::Preconfirmed(preconfirmed) = &mut new_head {
                        preconfirmed.refresh_with_candidates();
                    }

                    // Update statuses for all transactions in new_head.
                    match &new_head {
                        MadaraBlockView::Confirmed(confirmed) => {
                            self.update_block_transaction_statuses(
                                &new_head,
                                confirmed.get_block_info()?.tx_hashes.iter().cloned().enumerate(),
                                &mut removed,
                            )?;

                            // Mark the nonces from the state diff for update.
                            nonce_updates.extend(
                                confirmed.get_state_diff()?.nonces.iter().map(|n| (n.contract_address, n.nonce))
                            );
                        }
                        MadaraBlockView::Preconfirmed(preconfirmed) => {
                            // Executed transactions.
                            self.update_block_transaction_statuses(
                                &new_head,
                                preconfirmed.get_block_info().tx_hashes.iter().cloned().enumerate(),
                                &mut removed,
                            )?;
                            // Candidate transactions.
                            self.update_block_transaction_statuses(
                                &new_head,
                                preconfirmed.candidate_transactions().iter().enumerate().map(|(candidate_index, tx)| {
                                    (candidate_index + preconfirmed.num_executed_transactions(), tx.hash)
                                }),
                                &mut removed,
                            )?;

                            // Mark the nonces from the state diff for update.
                            nonce_updates.extend(
                                preconfirmed
                                    .borrow_content()
                                    .executed_transactions()
                                    .flat_map(|tx| tx.state_diff.nonces.iter()),
                            );
                        }
                    }

                    current_head = Some(new_head);
                }

                // Process blocks confirmed on l1. Avoid updates that are past the l2 tip though.
                new_head_on_l1 = l1_new_heads_subscription.next_block_view(),
                    if *l1_new_heads_subscription.current() < new_heads_subscription.current().latest_confirmed_block_n() =>
                {
                    tracing::debug!("Mempool task: new head on l1.");
                    let new_head_on_l1: MadaraBlockView<D> = new_head_on_l1.into();
                    self.update_block_transaction_statuses(
                        &new_head_on_l1,
                        new_head_on_l1.get_block_info()?.tx_hashes().iter().cloned().enumerate(),
                        &mut removed,
                    )?;
                }

                // Cancel condition.
                _ = ctx.cancelled() => {
                    return Ok(())
                }
            }

            tracing::debug!(
                "Mempool task: #nonce_updates={} #removed={}, put_back_into_mempool={put_back_into_mempool}.",
                nonce_updates.len(),
                removed.len()
            );

            // Update nonces
            if !nonce_updates.is_empty() {
                let mut guard = self.inner.write().await;
                let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();
                for (contract_address, account_nonce) in nonce_updates {
                    guard.update_account_nonce(
                        &contract_address.try_into().context("Invalid contract address")?,
                        &Nonce(account_nonce),
                        &mut removed_txs,
                    );
                }
            }

            // Update the mempool with the modifications.
            for (tx_hash, tx) in removed {
                if put_back_into_mempool {
                    // Try to add back to mempool.
                    if let Err(err) = self.accept_tx((*tx).clone()).await {
                        // Re-insertion may fail for various valid reasons: the tx has reached its TTL, the tx is a L1HandlerTransaction..
                        // TODO: it may fail because of tip-bump / eviction score. Maybe we shouldn't drop the tx in these cases?
                        tracing::debug!("Could not add transaction {:#x} back into mempool: {err:#}", tx.hash);
                    }
                } else {
                    // Drop the transaction entirely.
                    self.set_transaction_status(tx_hash, None);
                }
            }
        }
    }
}
