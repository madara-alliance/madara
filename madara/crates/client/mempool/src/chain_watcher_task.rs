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
use starknet_api::core::Nonce;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

fn is_preconfirmed_forward_advance(current_preconfirmed_n: Option<u64>, next_preconfirmed_n: Option<u64>) -> bool {
    matches!((current_preconfirmed_n, next_preconfirmed_n), (Some(current), Some(next)) if next > current)
}

struct ChainWatcherBranchEffects {
    potentially_removed: HashMap<Felt, Arc<ValidatedTransaction>>,
    executed_reinsert_suppressed: HashSet<Felt>,
    put_back_into_mempool: bool,
    nonce_updates: HashMap<Felt, Felt>,
    nonce_floors: HashMap<Felt, Felt>,
}

impl ChainWatcherBranchEffects {
    fn new() -> Self {
        Self {
            potentially_removed: HashMap::new(),
            executed_reinsert_suppressed: HashSet::new(),
            put_back_into_mempool: true,
            nonce_updates: HashMap::new(),
            nonce_floors: HashMap::new(),
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
        executed_reinsert_suppressed: &mut HashSet<Felt>,
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
            // On non-forward internal rewinds, executed descendant transactions are owned by the
            // explicit fallback replay pipeline. We still clear their stale preconfirmed status,
            // but we must not also reinsert them into the mempool or we can create duplicate /
            // gapped same-account queues.
            let tx_hash = *tx.transaction.receipt.transaction_hash();
            potentially_removed.insert(tx_hash, tx.to_validated().into());
            executed_reinsert_suppressed.insert(tx_hash);
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
        let current_tx_hashes = preconfirmed.get_block_info().tx_hashes;
        let first_new_tx_index = previous_num_txs.min(current_tx_hashes.len());

        if current_tx_hashes.len() < previous_num_txs {
            tracing::debug!(
                block_n = preconfirmed.block_number(),
                previous_tx_count = previous_num_txs,
                replacement_tx_count = current_tx_hashes.len(),
                "mempool_chain_watcher_reconciling_preconfirmed_content_replacement"
            );
        }

        self.update_preconfirmed_block_transaction_statuses(
            preconfirmed,
            current_tx_hashes.iter().cloned().enumerate().skip(first_new_tx_index),
            first_new_tx_index,
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
        if let MadaraBlockView::Confirmed(confirmed) = &new_head {
            let backend_internal_tip = self.backend.chain_head_state().internal_preconfirmed_tip;
            let confirmation_is_behind_internal_frontier = current_internal_frontier
                .as_ref()
                .is_some_and(|current| current.block_number() > confirmed.block_number())
                && backend_internal_tip.is_some_and(|tip| tip > confirmed.block_number());

            if confirmation_is_behind_internal_frontier {
                // The internal subscription still emits every confirmed block while execution runs ahead.
                // This is canonical progress below the speculative frontier, not a rewind of that frontier.
                self.update_block_transaction_statuses(
                    &new_head,
                    confirmed.get_block_info()?.tx_hashes.iter().cloned().enumerate(),
                    &mut effects.potentially_removed,
                )?;
                // This confirmation may lag newer speculative nonce updates, so it is a floor rather than an exact value.
                effects
                    .nonce_floors
                    .extend(confirmed.get_state_diff()?.nonces.iter().map(|n| (n.contract_address, n.nonce)));
                return Ok(());
            }
        }

        let current_preconfirmed_n =
            current_internal_frontier.as_ref().and_then(|v| v.as_preconfirmed()).map(|v| v.block_number());
        let next_preconfirmed_n = new_head.as_preconfirmed().map(|v| v.block_number());
        let preconfirmed_forward_advance = is_preconfirmed_forward_advance(current_preconfirmed_n, next_preconfirmed_n);

        // If the previous frontier was preconfirmed, mark potentially removed transactions and nonce rollback.
        self.collect_previous_preconfirmed_potentially_removed_transactions(
            current_internal_frontier.as_ref(),
            preconfirmed_forward_advance,
            &mut effects.potentially_removed,
            &mut effects.executed_reinsert_suppressed,
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

    async fn apply_nonce_updates(
        &self,
        nonce_updates: HashMap<Felt, Felt>,
        nonce_floors: HashMap<Felt, Felt>,
    ) -> anyhow::Result<()> {
        if nonce_updates.is_empty() && nonce_floors.is_empty() {
            return Ok(());
        }

        let mut guard = self.inner.write().await;
        let summary_before = guard.summary();
        let mut removed_txs = smallvec::SmallVec::<[ValidatedTransaction; 1]>::new();
        for (contract_address, account_nonce) in nonce_updates {
            let contract_address = contract_address.try_into().context("Invalid contract address")?;
            let new_nonce = Nonce(account_nonce);
            let old_nonce = guard.get_account_nonce(&contract_address).copied();
            guard.update_account_nonce(&contract_address, &new_nonce, &mut removed_txs);
            tracing::debug!(
                "mempool_chain_watcher_applied_nonce_update contract_address={contract_address:?} old_nonce={old_nonce:?} new_nonce={new_nonce:?} removed_txs_so_far={}",
                removed_txs.len(),
            );
        }
        for (contract_address, account_nonce) in nonce_floors {
            let contract_address = contract_address.try_into().context("Invalid contract address")?;
            let new_nonce = Nonce(account_nonce);
            let Some(old_nonce) = guard.get_account_nonce(&contract_address).copied() else { continue };
            if old_nonce >= new_nonce {
                continue;
            }
            guard.update_account_nonce(&contract_address, &new_nonce, &mut removed_txs);
            tracing::debug!(
                "mempool_chain_watcher_applied_confirmed_nonce_floor contract_address={contract_address:?} old_nonce={old_nonce:?} new_nonce={new_nonce:?} removed_txs_so_far={}",
                removed_txs.len(),
            );
        }
        let summary_after = guard.summary();
        tracing::debug!(
            "mempool_nonce_updates_applied summary_before={summary_before} summary_after={summary_after} removed_txs_total={}",
            removed_txs.len(),
        );
        self.metrics.record_mempool_state(&summary_after);

        Ok(())
    }

    async fn apply_potentially_removed_transactions(
        &self,
        potentially_removed: HashMap<Felt, Arc<ValidatedTransaction>>,
        put_back_into_mempool: bool,
        executed_reinsert_suppressed: HashSet<Felt>,
    ) {
        if !executed_reinsert_suppressed.is_empty() {
            tracing::debug!(
                suppressed_executed_reinsertions = executed_reinsert_suppressed.len(),
                "mempool_chain_watcher_suppressing_executed_reinsertion_on_rewind"
            );
        }
        // Update the mempool with the modifications.
        for (tx_hash, tx) in potentially_removed {
            if put_back_into_mempool && !executed_reinsert_suppressed.contains(&tx_hash) {
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

                // Canonical/internal frontier progress must not be starved by a busy preconfirmed
                // content watch. The new-head handler refreshes the full preconfirmed content, so
                // handling it first cannot lose transaction updates.
                new_head = new_heads_subscription.next_block_view() => {
                    tracing::debug!("Mempool task: new head.");
                    self.handle_new_internal_frontier(&mut current_internal_frontier, new_head, &mut effects)?;
                }

                // New transaction content on the current preconfirmed block.
                Some(preconfirmed) = OptionFuture::from(current_internal_frontier.as_mut().and_then(|v| v.as_preconfirmed_mut()).map(|v| async {
                    v.wait_until_outdated().await;
                    v
                })) => {
                    tracing::debug!("Mempool task: preconfirmed update.");
                    self.handle_preconfirmed_content_update(preconfirmed, &mut effects)?;
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
                "Mempool task: #nonce_updates={} #nonce_floors={} #potentially_removed={} #suppressed_executed_reinsertions={} put_back_into_mempool={}.",
                effects.nonce_updates.len(),
                effects.nonce_floors.len(),
                effects.potentially_removed.len(),
                effects.executed_reinsert_suppressed.len(),
                effects.put_back_into_mempool
            );

            self.apply_nonce_updates(effects.nonce_updates, effects.nonce_floors).await?;
            self.apply_potentially_removed_transactions(
                effects.potentially_removed,
                effects.put_back_into_mempool,
                effects.executed_reinsert_suppressed,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    // TODO(heemankv): The mempool behavior exercised on this branch has been validated manually
    // and is working, but newer persistence/recovery paths still need stronger automated coverage
    // here. Any future mempool behavior change should land with explicit tests.
    use super::{is_preconfirmed_forward_advance, ChainWatcherBranchEffects, Mempool, PreConfirmationStatus};
    use crate::MempoolConfig;
    use mc_db::{preconfirmed::PreconfirmedBlock, MadaraBlockView};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, Transaction};
    use mp_convert::Felt;
    use mp_state_update::{NonceUpdate, StateDiff};
    use mp_transactions::{validated::ValidatedTransaction, InvokeTransaction};
    use mp_utils::service::ServiceContext;
    use starknet_api::core::Nonce;
    use std::{collections::HashMap, sync::Arc, time::Duration};

    async fn backend_with_genesis() -> Arc<mc_db::MadaraBackend> {
        let backend = mc_db::MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
        let mut genesis = mc_devnet::ChainGenesisDescription::base_config().expect("base config");
        genesis.add_devnet_contracts(10).expect("devnet contracts");
        genesis.build_and_store(&backend).await.expect("genesis build");
        backend
    }

    fn invoke_with_nonce(account: Felt, nonce: Felt, hash: Felt) -> ValidatedTransaction {
        let mut tx = crate::tests::tx_account(account);
        tx.hash = hash;
        if let Transaction::Invoke(InvokeTransaction::V3(tx)) = &mut tx.transaction {
            tx.nonce = nonce;
        } else {
            panic!("expected invoke v3 test transaction")
        }
        tx
    }

    #[rstest::rstest]
    #[case(Some(42), Some(43), true)]
    #[case(Some(42), Some(44), true)]
    #[case(Some(42), Some(42), false)]
    #[case(Some(44), Some(42), false)]
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
        assert!(effects.executed_reinsert_suppressed.is_empty());
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
        assert!(effects.executed_reinsert_suppressed.is_empty());
        assert!(effects.nonce_updates.is_empty(), "forward advance should not rollback old executed nonces");
    }

    #[tokio::test]
    async fn confirmed_ancestor_advances_nonce_without_rewinding_internal_preconfirmed_frontier() {
        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
        let account = Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");
        let pending_tx = invoke_with_nonce(account, Felt::ONE, Felt::from(0xabcdu64));
        mempool.accept_tx(pending_tx.clone()).await.expect("future-nonce transaction should queue");
        assert!(!mempool.inner.read().await.has_ready_transactions());

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 1, ..Default::default() },
                    state_diff: StateDiff {
                        nonces: vec![NonceUpdate { contract_address: account, nonce: Felt::ONE }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                &[],
                false,
            )
            .expect("confirmed parent creation");

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader { block_number: 2, ..Default::default() },
                vec![],
                vec![],
            ))
            .expect("internal preconfirmed block creation");
        let mut internal_view = backend.block_view_on_preconfirmed(2).expect("internal preconfirmed block view");
        internal_view.refresh_with_candidates();

        let mut current_internal_frontier = Some(MadaraBlockView::Preconfirmed(internal_view));
        let confirmed_parent: MadaraBlockView<_> =
            backend.block_view_on_confirmed(1).expect("parent block should be confirmed").into();
        let mut effects = ChainWatcherBranchEffects::new();

        mempool
            .handle_new_internal_frontier(&mut current_internal_frontier, confirmed_parent, &mut effects)
            .expect("confirmed ancestor handling");

        let current = current_internal_frontier
            .as_ref()
            .and_then(MadaraBlockView::as_preconfirmed)
            .expect("internal preconfirmed frontier must remain active");
        assert_eq!(current.block_number(), 2);
        assert!(effects.potentially_removed.is_empty());
        assert!(effects.executed_reinsert_suppressed.is_empty());
        assert!(effects.nonce_updates.is_empty());
        assert_eq!(effects.nonce_floors.get(&account), Some(&Felt::ONE));

        mempool.apply_nonce_updates(effects.nonce_updates, effects.nonce_floors).await.expect("apply nonce floor");
        assert!(mempool.inner.read().await.has_ready_transactions(), "confirmed nonce must promote the queued tx");
        assert_eq!(
            mempool.inner.read().await.get_account_nonce(&account.try_into().unwrap()).copied(),
            Some(Nonce(Felt::ONE))
        );
        assert_eq!(mempool.get_consumer().await.next().expect("promoted transaction should pop").hash, pending_tx.hash);
    }

    #[tokio::test]
    async fn confirmed_nonce_floor_does_not_regress_speculative_nonce() {
        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend, MempoolConfig::default());
        let account = Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");
        let pending_tx = invoke_with_nonce(account, Felt::from(2u64), Felt::from(0xabceu64));
        mempool.accept_tx(pending_tx.clone()).await.expect("future-nonce transaction should queue");
        mempool
            .apply_nonce_updates([(account, Felt::from(2u64))].into(), HashMap::new())
            .await
            .expect("apply speculative nonce");
        mempool
            .apply_nonce_updates(HashMap::new(), [(account, Felt::ONE)].into())
            .await
            .expect("apply older confirmed floor");
        assert_eq!(
            mempool.inner.read().await.get_account_nonce(&account.try_into().unwrap()).copied(),
            Some(Nonce(Felt::from(2u64))),
            "an older confirmation must not regress the speculative account nonce"
        );
        assert_eq!(
            mempool.get_consumer().await.next().expect("speculatively promoted transaction should remain ready").hash,
            pending_tx.hash
        );
    }

    #[tokio::test]
    async fn devnet_chain_watcher_promotes_nonce_from_confirmed_parent_behind_speculative_frontier() {
        let backend = backend_with_genesis().await;
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
        let account = Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");
        let pending_tx = invoke_with_nonce(account, Felt::ONE, Felt::from(0xdef0u64));
        mempool.accept_tx(pending_tx.clone()).await.expect("future-nonce transaction should queue");

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
            .expect("first internal preconfirmed block");

        let frontier_marker = Arc::new(crate::tests::tx_account(Felt::from(0x1234u64)));
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader { block_number: 2, ..Default::default() },
                vec![],
                vec![frontier_marker.clone()],
            ))
            .expect("second internal preconfirmed block");

        let watcher = {
            let mempool = mempool.clone();
            tokio::spawn(async move { mempool.run_chain_watcher_task(ServiceContext::new()).await })
        };
        tokio::task::yield_now().await;

        tokio::time::timeout(Duration::from_secs(2), async {
            while !mempool.preconfirmed_transactions_statuses.contains_key(&frontier_marker.hash) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("chain watcher should reach the speculative frontier");

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 1, ..Default::default() },
                    state_diff: StateDiff {
                        nonces: vec![NonceUpdate { contract_address: account, nonce: Felt::ONE }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                &[],
                false,
            )
            .expect("canonical parent substitution");

        tokio::time::timeout(Duration::from_secs(2), async {
            while !mempool.inner.read().await.has_ready_transactions() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("confirmed parent nonce should promote the queued transaction");

        assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(2));
        assert_eq!(
            mempool.get_consumer().await.next().expect("promoted transaction should be consumable").hash,
            pending_tx.hash
        );
        watcher.abort();
    }

    #[tokio::test]
    async fn devnet_chain_watcher_promotes_nonce_when_preconfirmed_block_is_confirmed() {
        let backend = backend_with_genesis().await;
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
        let account = Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");
        let pending_tx = invoke_with_nonce(account, Felt::ONE, Felt::from(0xdef1u64));
        mempool.accept_tx(pending_tx.clone()).await.expect("future-nonce transaction should queue");

        let watcher = {
            let mempool = mempool.clone();
            tokio::spawn(async move { mempool.run_chain_watcher_task(ServiceContext::new()).await })
        };
        tokio::task::yield_now().await;

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
            .expect("preconfirmed block creation");
        tokio::task::yield_now().await;

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 1, ..Default::default() },
                    state_diff: StateDiff {
                        nonces: vec![NonceUpdate { contract_address: account, nonce: Felt::ONE }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                &[],
                false,
            )
            .expect("preconfirmed block confirmation");

        tokio::time::timeout(Duration::from_secs(2), async {
            while !mempool.inner.read().await.has_ready_transactions() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("confirmed nonce should promote the queued transaction");

        assert_eq!(
            mempool.get_consumer().await.next().expect("promoted transaction should be consumable").hash,
            pending_tx.hash
        );
        watcher.abort();
    }

    #[tokio::test]
    async fn devnet_chain_watcher_confirmation_is_not_starved_by_preconfirmed_updates() {
        let backend = backend_with_genesis().await;
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
        let account = Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");
        let pending_tx = invoke_with_nonce(account, Felt::ONE, Felt::from(0xdef2u64));
        mempool.accept_tx(pending_tx.clone()).await.expect("future-nonce transaction should queue");

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
            .expect("preconfirmed block creation");
        let preconfirmed = backend.block_view_on_preconfirmed(1).expect("runtime preconfirmed block").block().clone();

        let watcher = {
            let mempool = mempool.clone();
            tokio::spawn(async move { mempool.run_chain_watcher_task(ServiceContext::new()).await })
        };
        tokio::task::yield_now().await;

        let noisy_preconfirmed = preconfirmed.clone();
        let content_updates = tokio::spawn(async move {
            loop {
                noisy_preconfirmed.append(
                    std::iter::empty::<mc_db::preconfirmed::PreconfirmedExecutedTransaction>(),
                    std::iter::empty::<Arc<ValidatedTransaction>>(),
                );
                tokio::task::yield_now().await;
            }
        });

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 1, ..Default::default() },
                    state_diff: StateDiff {
                        nonces: vec![NonceUpdate { contract_address: account, nonce: Felt::ONE }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                &[],
                false,
            )
            .expect("preconfirmed block confirmation");

        tokio::time::timeout(Duration::from_secs(2), async {
            while !mempool.inner.read().await.has_ready_transactions() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("confirmed nonce must not be starved by preconfirmed content updates");

        assert_eq!(
            mempool.get_consumer().await.next().expect("promoted transaction should be consumable").hash,
            pending_tx.hash
        );
        content_updates.abort();
        watcher.abort();
    }

    #[tokio::test]
    async fn handle_new_internal_frontier_non_forward_suppresses_old_executed_reinsertion() {
        use mc_db::preconfirmed::PreconfirmedExecutedTransaction;
        use mp_block::TransactionWithReceipt;
        use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
        use mp_state_update::TransactionStateUpdate;
        use std::collections::HashMap;

        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());

        let mut executed_tx = crate::tests::tx_account(Felt::from(0x3333u64));
        let executed_hash = Felt::ZERO;
        executed_tx.hash = executed_hash;
        if let mp_block::Transaction::Invoke(mp_transactions::InvokeTransaction::V3(tx)) = &mut executed_tx.transaction
        {
            tx.nonce = Felt::from(7u64);
        }

        let executed = PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: executed_tx.transaction.clone(),
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    transaction_hash: executed_hash,
                    ..Default::default()
                }),
            },
            state_diff: TransactionStateUpdate {
                storage_diffs: Default::default(),
                contract_class_hashes: Default::default(),
                declared_classes: Default::default(),
                nonces: HashMap::from([(Felt::from(0x3333u64), Felt::from(8u64))]),
            },
            declared_class: None,
            arrived_at: executed_tx.arrived_at,
            paid_fee_on_l1: None,
        };
        let old_candidate = Arc::new(crate::tests::tx_account(Felt::from(0x3333u64)));

        let old_block = Arc::new(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 0, ..Default::default() },
            vec![executed],
            vec![old_candidate.clone()],
        ));
        let mut old_view = mc_db::MadaraPreconfirmedBlockView::new(backend.clone(), old_block);
        old_view.refresh_with_candidates();

        let mut current_internal_frontier = Some(MadaraBlockView::Preconfirmed(old_view));
        let new_head: MadaraBlockView<_> = mc_db::MadaraPreconfirmedBlockView::new(
            backend,
            Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() })),
        )
        .into();

        let mut effects = ChainWatcherBranchEffects::new();
        mempool
            .handle_new_internal_frontier(&mut current_internal_frontier, new_head, &mut effects)
            .expect("new internal frontier handling");

        assert!(effects.potentially_removed.contains_key(&executed_hash));
        assert!(effects.potentially_removed.contains_key(&old_candidate.hash));
        assert!(effects.executed_reinsert_suppressed.contains(&executed_hash));
        assert!(!effects.executed_reinsert_suppressed.contains(&old_candidate.hash));
        assert_eq!(
            effects.nonce_updates.get(&Felt::from(0x3333u64)).copied(),
            Some(Felt::ZERO),
            "rewind should roll executed nonce back to parent value"
        );
    }

    #[tokio::test]
    async fn skipped_preconfirmed_frontier_is_forward_progress_and_does_not_roll_back_nonce() {
        use mc_db::preconfirmed::PreconfirmedExecutedTransaction;
        use mp_block::TransactionWithReceipt;
        use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
        use mp_state_update::TransactionStateUpdate;
        use std::collections::HashMap;

        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend.clone(), MempoolConfig::default());
        let account = Felt::from(0x3333u64);
        let successor = invoke_with_nonce(account, Felt::from(8u64), Felt::from(0x8000u64));
        mempool.accept_tx(successor.clone()).await.expect("future-nonce successor should queue");
        mempool
            .apply_nonce_updates(HashMap::from([(account, Felt::from(8u64))]), HashMap::new())
            .await
            .expect("executed transaction should make its successor ready");

        let executed_tx = invoke_with_nonce(account, Felt::from(7u64), Felt::from(0x7000u64));
        let executed_hash = executed_tx.hash;
        let executed = PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: executed_tx.transaction.clone(),
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    transaction_hash: executed_hash,
                    ..Default::default()
                }),
            },
            state_diff: TransactionStateUpdate {
                storage_diffs: Default::default(),
                contract_class_hashes: Default::default(),
                declared_classes: Default::default(),
                nonces: HashMap::from([(account, Felt::from(8u64))]),
            },
            declared_class: None,
            arrived_at: executed_tx.arrived_at,
            paid_fee_on_l1: None,
        };

        let old_block = Arc::new(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 1, ..Default::default() },
            vec![executed],
            vec![],
        ));
        let mut old_view = mc_db::MadaraPreconfirmedBlockView::new(backend.clone(), old_block);
        old_view.refresh_with_candidates();
        let mut current_internal_frontier = Some(MadaraBlockView::Preconfirmed(old_view));

        // Watch notifications may coalesce, so block #2 can legitimately be skipped.
        let new_head: MadaraBlockView<_> = mc_db::MadaraPreconfirmedBlockView::new(
            backend,
            Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 3, ..Default::default() })),
        )
        .into();
        let mut effects = ChainWatcherBranchEffects::new();

        mempool
            .handle_new_internal_frontier(&mut current_internal_frontier, new_head, &mut effects)
            .expect("skipped forward frontier handling");

        assert!(effects.executed_reinsert_suppressed.is_empty());
        assert!(!effects.potentially_removed.contains_key(&executed_hash));
        assert!(!effects.nonce_updates.contains_key(&account), "forward progress must not synthesize a nonce rollback");
        mempool
            .apply_nonce_updates(effects.nonce_updates, effects.nonce_floors)
            .await
            .expect("apply skipped-frontier effects");
        assert_eq!(mempool.get_consumer().await.next().expect("successor should remain ready").hash, successor.hash);
    }

    #[tokio::test]
    async fn apply_potentially_removed_transactions_does_not_reinsert_suppressed_executed_txs() {
        use starknet_api::transaction::TransactionHash;
        use std::collections::{HashMap, HashSet};

        let backend = backend_with_genesis().await;
        let mempool = Mempool::new(backend, MempoolConfig::default());

        let executed = Arc::new(crate::tests::tx_account(Felt::from(0x4444u64)));
        let candidate = Arc::new(crate::tests::tx_account(Felt::from(0x5555u64)));
        let executed_hash = executed.hash;
        let candidate_hash = candidate.hash;

        let mut potentially_removed = HashMap::new();
        potentially_removed.insert(executed_hash, executed.clone());
        potentially_removed.insert(candidate_hash, candidate.clone());

        let mut executed_reinsert_suppressed = HashSet::new();
        executed_reinsert_suppressed.insert(executed_hash);

        mempool.apply_potentially_removed_transactions(potentially_removed, true, executed_reinsert_suppressed).await;

        let inner = mempool.inner.read().await;
        assert!(!inner.contains_tx_by_hash(&TransactionHash(executed_hash)));
        assert!(inner.contains_tx_by_hash(&TransactionHash(candidate_hash)));
    }
}
