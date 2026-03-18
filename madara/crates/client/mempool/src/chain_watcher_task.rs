use crate::{
    transaction_status::{PreConfirmationStatus, TransactionStatus},
    Mempool,
};
use anyhow::Context;
use futures::future::OptionFuture;
use mc_db::{MadaraBlockView, MadaraPreconfirmedBlockView, MadaraStorageRead, MadaraStorageWrite};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use mp_utils::service::ServiceContext;
use std::{collections::HashMap, sync::Arc};

fn is_preconfirmed_forward_advance(current_preconfirmed_n: Option<u64>, next_preconfirmed_n: Option<u64>) -> bool {
    matches!(
        (current_preconfirmed_n, next_preconfirmed_n),
        (Some(current), Some(next)) if current.checked_add(1) == Some(next)
    )
}

struct ChainWatcherBranchEffects {
    potentially_removed: HashMap<Felt, Arc<ValidatedTransaction>>,
    put_back_into_mempool: bool,
    nonce_updates: HashMap<Felt, Felt>,
}

impl ChainWatcherBranchEffects {
    fn new() -> Self {
        Self { potentially_removed: HashMap::new(), put_back_into_mempool: true, nonce_updates: HashMap::new() }
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

    fn update_preconfirmed_block_transaction_statuses(
        &self,
        preconfirmed: &MadaraPreconfirmedBlockView<D>,
        executed_iter: impl IntoIterator<Item = (usize, Felt)>,
        nonce_skip: usize,
        potentially_removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
        nonce_updates: &mut HashMap<Felt, Felt>,
    ) -> anyhow::Result<()> {
        let view: MadaraBlockView<D> = preconfirmed.clone().into();

        // Executed transactions.
        self.update_block_transaction_statuses(&view, executed_iter, potentially_removed)?;
        // Candidate transactions.
        self.update_block_transaction_statuses(
            &view,
            preconfirmed
                .candidate_transactions()
                .iter()
                .enumerate()
                .map(|(candidate_index, tx)| (candidate_index + preconfirmed.num_executed_transactions(), tx.hash)),
            potentially_removed,
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

    fn mark_candidate_transactions_as_potentially_removed(
        &self,
        preconfirmed: &MadaraPreconfirmedBlockView<D>,
        potentially_removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
    ) {
        for tx in preconfirmed.candidate_transactions() {
            potentially_removed.insert(tx.hash, tx.clone());
        }
    }

    fn collect_previous_preconfirmed_potentially_removed_transactions(
        &self,
        current_internal_frontier: Option<&MadaraBlockView<D>>,
        preconfirmed_forward_advance: bool,
        potentially_removed: &mut HashMap<Felt, Arc<ValidatedTransaction>>,
        nonce_updates: &mut HashMap<Felt, Felt>,
    ) -> anyhow::Result<()> {
        let Some(preconfirmed) = current_internal_frontier.and_then(|v| v.as_preconfirmed()) else {
            return Ok(());
        };

        if preconfirmed_forward_advance {
            // On normal forward progress to the next preconfirmed block, previous executed transactions
            // must stay out of the mempool and keep their preconfirmed status.
            self.mark_candidate_transactions_as_potentially_removed(preconfirmed, potentially_removed);
            return Ok(());
        }

        let view_on_parent = preconfirmed.state_view_on_parent();
        for tx in preconfirmed.borrow_content().executed_transactions() {
            // Re-convert PreconfirmedExecutedTransaction to ValidatedTransaction.
            potentially_removed.insert(*tx.transaction.receipt.transaction_hash(), tx.to_validated().into());
            // Rollback the contract nonce to what it was before the transaction.
            for key in tx.state_diff.nonces.keys() {
                nonce_updates.insert(
                    *key,
                    // Get from db.
                    view_on_parent.get_contract_nonce(key)?.unwrap_or(Felt::ZERO),
                );
            }
        }
        self.mark_candidate_transactions_as_potentially_removed(preconfirmed, potentially_removed);

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
        )?;

        Ok(())
    }

    /// Branch #2:
    /// Process a new internal L2 frontier item (`Confirmed` or internal `Preconfirmed`).
    fn handle_new_internal_frontier(
        &self,
        current_internal_frontier: &mut Option<MadaraBlockView<D>>,
        mut new_head: MadaraBlockView<D>,
        effects: &mut ChainWatcherBranchEffects,
    ) -> anyhow::Result<()> {
        let current_preconfirmed_n =
            current_internal_frontier.as_ref().and_then(|v| v.as_preconfirmed()).map(|v| v.block_number());
        let next_preconfirmed_n = new_head.as_preconfirmed().map(|v| v.block_number());
        let preconfirmed_forward_advance = is_preconfirmed_forward_advance(current_preconfirmed_n, next_preconfirmed_n);

        // If the previous frontier was preconfirmed, mark potentially removed transactions and nonce rollback.
        self.collect_previous_preconfirmed_potentially_removed_transactions(
            current_internal_frontier.as_ref(),
            preconfirmed_forward_advance,
            &mut effects.potentially_removed,
            &mut effects.nonce_updates,
        )?;

        if let MadaraBlockView::Preconfirmed(preconfirmed) = &mut new_head {
            preconfirmed.refresh_with_candidates();
        }

        // Update statuses/nonces for transactions in the new frontier.
        match &new_head {
            MadaraBlockView::Confirmed(confirmed) => {
                self.update_block_transaction_statuses(
                    &new_head,
                    confirmed.get_block_info()?.tx_hashes.iter().cloned().enumerate(),
                    &mut effects.potentially_removed,
                )?;

                effects
                    .nonce_updates
                    .extend(confirmed.get_state_diff()?.nonces.iter().map(|n| (n.contract_address, n.nonce)));
            }
            MadaraBlockView::Preconfirmed(preconfirmed) => {
                self.update_preconfirmed_block_transaction_statuses(
                    preconfirmed,
                    preconfirmed.get_block_info().tx_hashes.iter().cloned().enumerate(),
                    0,
                    &mut effects.potentially_removed,
                    &mut effects.nonce_updates,
                )?;
            }
        }

        *current_internal_frontier = Some(new_head);
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
        )?;
        Ok(())
    }

    async fn apply_nonce_updates(&self, nonce_updates: HashMap<Felt, Felt>) -> anyhow::Result<()> {
        self.update_account_nonces(nonce_updates).await
    }

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
    /// ## Why this task keeps a single frontier cursor
    /// The backend has multiple canonical head fields (confirmed tip, external preconfirmed tip,
    /// internal preconfirmed tip). This task intentionally keeps one local cursor,
    /// `current_internal_frontier`, which means "last L2 frontier item already processed by mempool".
    /// It is not the canonical chain head; it is a processing cursor used to compute old->new deltas.
    ///
    /// ## Flow
    /// 1. Subscribe to internal L2 frontier updates (`Confirmed` + internal `Preconfirmed`) and L1 confirmations.
    /// 2. Initialize `current_internal_frontier` from the internal-head subscription.
    /// 3. In each loop iteration, process exactly one event branch:
    ///    - preconfirmed content update on the current preconfirmed frontier (new executed txs/candidates),
    ///    - new internal frontier item (`Confirmed` or `Preconfirmed`),
    ///    - new L1 confirmation for an already-known L2 confirmed block.
    /// 4. Accumulate nonce updates and potentially removed transactions for that event, then apply:
    ///    - nonce updates to inner mempool account state,
    ///    - tx reinsertion or drop decisions,
    ///    - status publication updates.
    ///
    /// This task updates preconfirmed/confirmed/L1 transaction statuses and is also responsible
    /// for putting reverted preconfirmed transactions back into the mempool when appropriate.
    pub(super) async fn run_chain_watcher_task(&self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        let mut l1_new_heads_subscription = self.backend.subscribe_new_l1_confirmed_heads();

        let mut new_heads_subscription =
            self.backend.subscribe_internal_heads(mc_db::subscription::SubscribeNewBlocksTag::Preconfirmed);
        // Start returning heads from the next block after the latest confirmed block (inclusive).
        new_heads_subscription
            .set_start_from(self.backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(/* genesis */ 0));

        // Last internal L2 frontier item already processed by this task.
        let mut current_internal_frontier = new_heads_subscription.current_block_view();

        loop {
            // When the pre-confirmed block changes, we need to put all potentially removed transactions back into the mempool.
            // However, we don't want to put them right away: for example, if the pre-confirmed block became confirmed, we don't want to insert
            // the transactions back into the mempool just to remove them right away to mark them confirmed. We use this map to track this.
            let mut effects = ChainWatcherBranchEffects::new();

            tokio::select! {
                biased;

                // Preconfirmed block new tx. We process this first to make sure we don't miss transactions.
                Some(preconfirmed) = OptionFuture::from(current_internal_frontier.as_mut().and_then(|v| v.as_preconfirmed_mut()).map(|v| async {
                    v.wait_until_outdated().await;
                    v
                })) => {
                    tracing::debug!("Mempool task: preconfirmed update.");
                    self.handle_preconfirmed_content_update(preconfirmed, &mut effects)?;
                }

                // New block on l2: either confirmed or pre-confirmed.
                new_head = new_heads_subscription.next_block_view() => {
                    tracing::debug!("Mempool task: new head.");
                    self.handle_new_internal_frontier(&mut current_internal_frontier, new_head, &mut effects)?;
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
                "Mempool task: #nonce_updates={} #potentially_removed={}, put_back_into_mempool={}.",
                effects.nonce_updates.len(),
                effects.potentially_removed.len(),
                effects.put_back_into_mempool
            );

            self.apply_nonce_updates(effects.nonce_updates).await?;
            self.apply_potentially_removed_transactions(effects.potentially_removed, effects.put_back_into_mempool)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_preconfirmed_forward_advance, ChainWatcherBranchEffects, Mempool, PreConfirmationStatus};
    use crate::MempoolConfig;
    use mc_db::{preconfirmed::PreconfirmedBlock, MadaraBlockView};
    use mp_block::header::PreconfirmedHeader;
    use mp_convert::Felt;
    use std::sync::Arc;

    async fn backend_with_genesis() -> Arc<mc_db::MadaraBackend> {
        let backend = mc_db::MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
        let mut genesis = mc_devnet::ChainGenesisDescription::base_config().expect("base config");
        genesis.add_devnet_contracts(10).expect("devnet contracts");
        genesis.build_and_store(&backend).await.expect("genesis build");
        backend
    }

    #[rstest::rstest]
    #[case(Some(42), Some(43), true)]
    #[case(Some(42), Some(42), false)]
    #[case(Some(42), Some(44), false)]
    #[case(Some(42), None, false)]
    #[case(None, Some(0), false)]
    #[case(None, None, false)]
    fn detect_forward_preconfirmed_advance(
        #[case] current_preconfirmed_n: Option<u64>,
        #[case] next_preconfirmed_n: Option<u64>,
        #[case] expected: bool,
    ) {
        assert_eq!(is_preconfirmed_forward_advance(current_preconfirmed_n, next_preconfirmed_n), expected);
    }

    #[tokio::test]
    async fn handle_preconfirmed_content_update_disables_reinsertion_and_sets_candidate_status() {
        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());

        let candidate = Arc::new(crate::tests::tx_account(Felt::from(0x1234u64)));
        let block = Arc::new(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 0, ..Default::default() },
            vec![],
            vec![candidate.clone()],
        ));
        let mut preconfirmed_view = mc_db::MadaraPreconfirmedBlockView::new(backend, block);
        let mut effects = ChainWatcherBranchEffects::new();

        mempool
            .handle_preconfirmed_content_update(&mut preconfirmed_view, &mut effects)
            .expect("preconfirmed content update");

        assert!(!effects.put_back_into_mempool, "candidates branch should drop potentially removed txs");
        assert!(effects.potentially_removed.is_empty(), "candidate is still present in refreshed view");

        let status = mempool
            .preconfirmed_transactions_statuses
            .get(&candidate.hash)
            .map(|status| status.clone())
            .expect("candidate status must be tracked");
        assert!(matches!(status, PreConfirmationStatus::Candidate { transaction_index: 0, .. }));
    }

    #[tokio::test]
    async fn handle_new_internal_frontier_non_forward_marks_old_candidates_potentially_removed() {
        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());

        let old_candidate = Arc::new(crate::tests::tx_account(Felt::from(0x5678u64)));
        let old_block = Arc::new(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 0, ..Default::default() },
            vec![],
            vec![old_candidate.clone()],
        ));
        let mut old_view = mc_db::MadaraPreconfirmedBlockView::new(backend.clone(), old_block);
        old_view.refresh_with_candidates();

        let mut current_internal_frontier = Some(MadaraBlockView::Preconfirmed(old_view));
        // Same block number => non-forward preconfirmed transition.
        let new_head: MadaraBlockView<_> = mc_db::MadaraPreconfirmedBlockView::new(
            backend.clone(),
            Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() })),
        )
        .into();

        let mut effects = ChainWatcherBranchEffects::new();
        mempool
            .handle_new_internal_frontier(&mut current_internal_frontier, new_head, &mut effects)
            .expect("new internal frontier handling");

        assert!(effects.potentially_removed.contains_key(&old_candidate.hash));
        assert!(effects.nonce_updates.is_empty());
    }

    #[tokio::test]
    async fn handle_new_internal_frontier_forward_only_marks_old_candidates_potentially_removed() {
        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());

        let old_candidate = Arc::new(crate::tests::tx_account(Felt::from(0x9abcu64)));
        let old_block = Arc::new(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 0, ..Default::default() },
            vec![],
            vec![old_candidate.clone()],
        ));
        let mut old_view = mc_db::MadaraPreconfirmedBlockView::new(backend.clone(), old_block);
        old_view.refresh_with_candidates();

        let mut current_internal_frontier = Some(MadaraBlockView::Preconfirmed(old_view));
        let new_head: MadaraBlockView<_> = mc_db::MadaraPreconfirmedBlockView::new(
            backend,
            Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() })),
        )
        .into();

        let mut effects = ChainWatcherBranchEffects::new();
        mempool
            .handle_new_internal_frontier(&mut current_internal_frontier, new_head, &mut effects)
            .expect("forward internal frontier handling");

        assert!(effects.potentially_removed.contains_key(&old_candidate.hash));
        assert!(effects.nonce_updates.is_empty(), "forward advance should not rollback old executed nonces");
    }
}
