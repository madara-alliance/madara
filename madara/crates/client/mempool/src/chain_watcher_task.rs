use crate::{
    transaction_status::{PreConfirmationStatus, TransactionStatus},
    Mempool, NonceUpdateMode,
};
use anyhow::Context;
use futures::future::OptionFuture;
use mc_db::{MadaraBlockView, MadaraPreconfirmedBlockView, MadaraStorageRead, MadaraStorageWrite};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use mp_utils::service::ServiceContext;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

/// Confirmation advances independently of the executed blocks still awaiting finalization.
struct ChainWatcherState<D: MadaraStorageRead> {
    confirmed_tip: Option<u64>,
    preconfirmed: BTreeMap<u64, TrackedPreconfirmed<D>>,
}

/// Keeps each block's latest nonce per account without rescanning its transactions on confirmation.
struct TrackedPreconfirmed<D: MadaraStorageRead> {
    view: MadaraPreconfirmedBlockView<D>,
    nonces: HashMap<Felt, Felt>,
}

impl<D: MadaraStorageRead> TrackedPreconfirmed<D> {
    /// Persisted blocks can be reconstructed into new Arcs while recovery is in progress.
    /// Their executed prefix identifies continuity independently of allocation identity.
    fn continues_in(&self, current: &MadaraPreconfirmedBlockView<D>) -> bool {
        Arc::ptr_eq(self.view.block(), current.block())
            || (self.view.block().header == current.block().header
                && current.get_block_info().tx_hashes.starts_with(&self.view.get_block_info().tx_hashes))
    }

    fn new(view: MadaraPreconfirmedBlockView<D>) -> Self {
        let nonces = view
            .borrow_content()
            .executed_transactions()
            .flat_map(|tx| tx.state_diff.nonces.iter().map(|(address, nonce)| (*address, *nonce)))
            .collect();
        Self { view, nonces }
    }
}

impl<D: MadaraStorageRead> ChainWatcherState<D> {
    fn new(confirmed_tip: Option<u64>) -> Self {
        Self { confirmed_tip, preconfirmed: BTreeMap::new() }
    }
}

struct ChainWatcherBranchEffects {
    potentially_removed: HashMap<Felt, Arc<ValidatedTransaction>>,
    put_back_into_mempool: bool,
    nonce_updates: HashMap<Felt, Felt>,
    nonce_update_mode: NonceUpdateMode,
    confirmed_tx_hashes: Vec<Felt>,
}

impl ChainWatcherBranchEffects {
    /// Creates an empty per-event effect accumulator with reinsertion enabled.
    /// Individual watcher branches may disable reinsertion before effects are applied.
    fn new() -> Self {
        Self {
            potentially_removed: HashMap::new(),
            put_back_into_mempool: true,
            nonce_updates: HashMap::new(),
            nonce_update_mode: NonceUpdateMode::Advance,
            confirmed_tx_hashes: Vec::new(),
        }
    }
}

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
        potentially_removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
        confirmed_tx_hashes: &mut Vec<Felt>,
    ) -> anyhow::Result<()> {
        let is_on_l1 = view.is_on_l1();
        for (tx_index, tx_hash) in iter {
            potentially_removed.remove(&tx_hash); // The transaction is still part of the current frontier.

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
                confirmed_tx_hashes.push(tx_hash);
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

    /// Updates executed and candidate statuses for one preconfirmed block view.
    /// Newly observed nonce writes and confirmed hashes are accumulated for deferred application.
    fn update_preconfirmed_block_transaction_statuses(
        &self,
        preconfirmed: &MadaraPreconfirmedBlockView<D>,
        executed_iter: impl IntoIterator<Item = (usize, Felt)>,
        nonce_skip: usize,
        potentially_removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
        nonce_updates: &mut HashMap<Felt, Felt>,
        confirmed_tx_hashes: &mut Vec<Felt>,
    ) -> anyhow::Result<()> {
        let view: MadaraBlockView<D> = preconfirmed.clone().into();

        // Executed transactions.
        self.update_block_transaction_statuses(&view, executed_iter, potentially_removed, confirmed_tx_hashes)?;
        // Candidate transactions.
        self.update_block_transaction_statuses(
            &view,
            preconfirmed
                .candidate_transactions()
                .iter()
                .enumerate()
                .map(|(candidate_index, tx)| (candidate_index + preconfirmed.num_executed_transactions(), tx.hash)),
            potentially_removed,
            confirmed_tx_hashes,
        )?;

        // Mark the nonces from the state diff for update.
        nonce_updates.extend(
            preconfirmed
                .borrow_content()
                .executed_transactions()
                .skip(nonce_skip)
                .flat_map(|tx| tx.state_diff.nonces.iter()),
        );

        Ok(())
    }

    /// Adds every candidate from the previous frontier to the pending-removal set.
    /// A later view update removes candidates that remain present before effects are applied.
    fn mark_candidate_transactions_as_potentially_removed(
        &self,
        preconfirmed: &MadaraPreconfirmedBlockView<D>,
        potentially_removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
    ) {
        for tx in preconfirmed.candidate_transactions() {
            potentially_removed.insert(tx.hash, tx.clone());
        }
    }

    /// Only blocks actually replaced or confirmed can lose executed transactions.
    fn collect_removed_preconfirmed(
        &self,
        preconfirmed: &MadaraPreconfirmedBlockView<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) {
        for tx in preconfirmed.borrow_content().executed_transactions() {
            effects.potentially_removed.insert(*tx.transaction.receipt.transaction_hash(), tx.to_validated().into());
            // Values are resolved against the remaining chain after the transition.
            effects.nonce_updates.extend(tx.state_diff.nonces.keys().map(|key| (*key, Felt::ZERO)));
        }
        self.mark_candidate_transactions_as_potentially_removed(preconfirmed, &mut effects.potentially_removed);
    }

    /// Resolves affected account nonces from confirmed state plus every retained execution layer.
    /// A confirmation behind runahead must not overwrite a nonce written by a later block.
    fn resolve_nonce_updates(
        &self,
        state: &ChainWatcherState<D>,
        nonce_updates: &mut HashMap<Felt, Felt>,
    ) -> anyhow::Result<()> {
        let confirmed_tip = self.backend.latest_confirmed_block_n();
        for (address, nonce) in nonce_updates.iter_mut() {
            *nonce = match confirmed_tip {
                Some(block_n) => self.backend.db.get_contract_nonce_at(block_n, address)?.unwrap_or(Felt::ZERO),
                None => Felt::ZERO,
            };
        }
        for pending in state
            .preconfirmed
            .values()
            .filter(|pending| confirmed_tip.is_none_or(|tip| pending.view.block_number() > tip))
        {
            for (address, update) in nonce_updates.iter_mut() {
                if let Some(nonce) = pending.nonces.get(address) {
                    *update = *nonce;
                }
            }
        }
        Ok(())
    }

    /// Retained execution layers protect transactions even when their earlier candidate is retired.
    fn resolve_branch_effects(
        &self,
        state: &ChainWatcherState<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) -> anyhow::Result<()> {
        self.resolve_nonce_updates(state, &mut effects.nonce_updates)?;
        if effects.potentially_removed.is_empty() {
            return Ok(());
        }
        for pending in state.preconfirmed.values() {
            let view = &pending.view;
            for hash in &view.get_block_info().tx_hashes {
                effects.potentially_removed.remove(hash);
            }
            for tx in view.candidate_transactions() {
                effects.potentially_removed.remove(&tx.hash);
            }
        }
        Ok(())
    }

    /// Branch #1:
    /// Update statuses/nonces when the current internal preconfirmed block receives new content.
    fn handle_preconfirmed_content_update(
        &self,
        preconfirmed: &mut MadaraPreconfirmedBlockView<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) -> anyhow::Result<()> {
        // Candidates that were not executed are most likely rejected transactions.
        // Do not reinsert them, or they can endlessly cycle mempool -> block builder -> mempool.
        effects.put_back_into_mempool = false;
        self.mark_candidate_transactions_as_potentially_removed(preconfirmed, &mut effects.potentially_removed);

        let previous_num_txs = preconfirmed.num_executed_transactions();
        preconfirmed.refresh_with_candidates();

        self.update_preconfirmed_block_transaction_statuses(
            preconfirmed,
            preconfirmed.get_block_info().tx_hashes[previous_num_txs..].iter().cloned().enumerate(),
            previous_num_txs,
            &mut effects.potentially_removed,
            &mut effects.nonce_updates,
            &mut effects.confirmed_tx_hashes,
        )?;

        Ok(())
    }

    /// Processes confirmations without replacing the independent execution frontier.
    fn handle_new_internal_frontier(
        &self,
        state: &mut ChainWatcherState<D>,
        new_head: MadaraBlockView<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) -> anyhow::Result<()> {
        match new_head {
            MadaraBlockView::Confirmed(confirmed) => {
                let block_n = confirmed.block_number();
                if let Some(previous) = state.preconfirmed.remove(&block_n) {
                    self.collect_removed_preconfirmed(&previous.view, effects);
                }
                let tx_hashes = confirmed.get_block_info()?.tx_hashes;
                effects
                    .nonce_updates
                    .extend(confirmed.get_state_diff()?.nonces.into_iter().map(|n| (n.contract_address, n.nonce)));
                self.update_block_transaction_statuses(
                    &confirmed.into(),
                    tx_hashes.into_iter().enumerate(),
                    &mut effects.potentially_removed,
                    &mut effects.confirmed_tx_hashes,
                )?;
                state.confirmed_tip = Some(block_n);
            }
            MadaraBlockView::Preconfirmed(_) => self.update_execution_frontier(state, effects)?,
        }
        Ok(())
    }

    /// Refreshes the entire unconfirmed suffix so coalesced notifications cannot skip executions.
    fn update_execution_frontier(
        &self,
        state: &mut ChainWatcherState<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) -> anyhow::Result<()> {
        let (head, views) = self.backend.internal_preconfirmed_views()?;
        let canonical: BTreeMap<_, _> = views.into_iter().map(|view| (view.block_number(), view)).collect();
        let first_replaced = state.preconfirmed.iter().find_map(|(&n, previous)| {
            if head.confirmed_tip.is_some_and(|confirmed| n <= confirmed) {
                // Its confirmation is still queued in the ordered subscription.
                return None;
            }
            canonical.get(&n).is_none_or(|current| !previous.continues_in(current)).then_some(n)
        });
        if let Some(first_replaced) = first_replaced {
            effects.nonce_update_mode = NonceUpdateMode::Replace;
            for previous in state.preconfirmed.split_off(&first_replaced).into_values() {
                self.collect_removed_preconfirmed(&previous.view, effects);
            }
        }

        for (n, mut current) in canonical {
            current.refresh_with_candidates();
            if let Some(previous) = state.preconfirmed.get(&n) {
                if previous.view == current
                    && previous
                        .view
                        .candidate_transactions()
                        .iter()
                        .map(|tx| tx.hash)
                        .eq(current.candidate_transactions().iter().map(|tx| tx.hash))
                {
                    continue;
                }
                self.mark_candidate_transactions_as_potentially_removed(
                    &previous.view,
                    &mut effects.potentially_removed,
                );
            }
            self.update_preconfirmed_block_transaction_statuses(
                &current,
                current.get_block_info().tx_hashes.iter().copied().enumerate(),
                0,
                &mut effects.potentially_removed,
                &mut effects.nonce_updates,
                &mut effects.confirmed_tx_hashes,
            )?;
            state.preconfirmed.insert(n, TrackedPreconfirmed::new(current));
        }
        Ok(())
    }

    /// Branch #3:
    /// Apply L1 finality updates for already-known L2 confirmed blocks.
    fn handle_new_l1_confirmation(
        &self,
        new_head_on_l1: MadaraBlockView<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) -> anyhow::Result<()> {
        self.update_block_transaction_statuses(
            &new_head_on_l1,
            new_head_on_l1.get_block_info()?.tx_hashes().iter().cloned().enumerate(),
            &mut effects.potentially_removed,
            &mut effects.confirmed_tx_hashes,
        )?;
        Ok(())
    }

    /// Applies nonce changes accumulated while processing one watcher event.
    /// Keeping this step separate ensures the event branch finishes inspecting storage first.
    async fn apply_nonce_updates(
        &self,
        nonce_updates: HashMap<Felt, Felt>,
        mode: NonceUpdateMode,
    ) -> anyhow::Result<()> {
        self.update_account_nonces(nonce_updates, mode).await
    }

    /// Requeues or drops transactions absent from the newly observed frontier.
    /// The selected branch controls reinsertion so rejected candidates cannot cycle indefinitely.
    async fn apply_potentially_removed_transactions(
        &self,
        potentially_removed: HashMap<Felt, Arc<ValidatedTransaction>>,
        put_back_into_mempool: bool,
    ) {
        // Update the mempool with the modifications.
        for (tx_hash, tx) in potentially_removed {
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

    /// Watches chain head/runtime updates and keeps mempool-facing transaction state in sync.
    ///
    /// Confirmed progress and the unconfirmed execution suffix have separate lifetimes.
    /// Confirmations retire one layer; only actual replacements requeue its executed transactions.
    pub(super) async fn run_chain_watcher_task(&self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        let mut l1_new_heads_subscription = self.backend.subscribe_new_l1_confirmed_heads();

        let mut new_heads_subscription =
            self.backend.subscribe_internal_heads(mc_db::subscription::SubscribeNewBlocksTag::Preconfirmed);
        let confirmed_tip = self.backend.latest_confirmed_block_n();
        new_heads_subscription.set_start_from(confirmed_tip.map_or(0, |n| n + 1));
        let mut state = ChainWatcherState::new(confirmed_tip);

        loop {
            // When the pre-confirmed block changes, we need to put all potentially removed transactions back into the mempool.
            // However, we don't want to put them right away: for example, if the pre-confirmed block became confirmed, we don't want to insert
            // the transactions back into the mempool just to remove them right away to mark them confirmed. We use this map to track this.
            let mut effects = ChainWatcherBranchEffects::new();

            tokio::select! {
                biased;

                // Preconfirmed block new tx. We process this first to make sure we don't miss transactions.
                Some(preconfirmed) = OptionFuture::from(state.preconfirmed.last_entry().map(|entry| entry.into_mut()).map(|v| async {
                    v.view.wait_until_outdated().await;
                    v
                })) => {
                    tracing::debug!("Mempool task: preconfirmed update.");
                    self.handle_preconfirmed_content_update(&mut preconfirmed.view, &mut effects)?;
                    preconfirmed.nonces.extend(effects.nonce_updates.iter().map(|(address, nonce)| (*address, *nonce)));
                }

                // New block on l2: either confirmed or pre-confirmed.
                new_head = new_heads_subscription.next_block_view() => {
                    tracing::debug!("Mempool task: new head.");
                    self.handle_new_internal_frontier(&mut state, new_head, &mut effects)?;
                }

                // Process blocks confirmed on l1. Avoid updates that are past the l2 tip though.
                new_head_on_l1 = l1_new_heads_subscription.next_block_view(),
                    if *l1_new_heads_subscription.current() < new_heads_subscription.current_confirmed_block_n() =>
                {
                    tracing::debug!("Mempool task: new head on l1.");
                    self.handle_new_l1_confirmation(new_head_on_l1.into(), &mut effects)?;
                }

                // Cancel condition.
                _ = ctx.cancelled() => {
                    return Ok(())
                }
            }

            tracing::debug!(
                "Mempool task: #nonce_updates={} #potentially_removed={} #confirmed_tx_hashes={}, put_back_into_mempool={}.",
                effects.nonce_updates.len(),
                effects.potentially_removed.len(),
                effects.confirmed_tx_hashes.len(),
                effects.put_back_into_mempool
            );

            self.resolve_branch_effects(&state, &mut effects)?;
            self.remove_saved_txs_by_hashes(effects.confirmed_tx_hashes);
            self.apply_nonce_updates(effects.nonce_updates, effects.nonce_update_mode).await?;
            self.apply_potentially_removed_transactions(effects.potentially_removed, effects.put_back_into_mempool)
                .await;
            self.metrics.record_preconfirmed_transaction_statuses(self.preconfirmed_transactions_statuses.len());
        }
    }
}

#[cfg(test)]
mod tests;
