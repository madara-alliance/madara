use super::backend::{
    ensure_parallel_merkle_revert_is_retained, ensure_reorg_target_root_matches, revert_single_trie, TrieLogHeads,
};
use super::*;

/// Immutable chain positions and block metadata resolved before a reorg mutates storage.
///
/// Keeping this preflight context together makes every destructive phase use the same target and source tips.
struct ReorgContext {
    target_block_n: u64,
    target_block_info: MadaraBlockInfo,
    current_tip: u64,
    current_tip_info: MadaraBlockInfo,
    had_preconfirmed_tip: bool,
}

/// L1 message cleanup and cursor state derived before a reorg mutates storage.
///
/// The plan is committed atomically with the canonical head so startup can resume safely after a crash.
struct L1RewindPlan {
    nonces_to_cleanup: Vec<u64>,
    rewind_from_l1_block: Option<u64>,
    sync_tip_after_revert: Option<u64>,
}

impl RocksDBStorage {
    /// Resolves and validates the source and target heads for a reorg request.
    ///
    /// No writes occur here, so every later failure still leaves the old confirmed head authoritative.
    fn prepare_reorg(&self, new_tip_block_hash: &Felt) -> Result<ReorgContext> {
        let target_block_n = self
            .inner
            .find_block_hash(new_tip_block_hash)
            .context("Finding target block for reorg")?
            .ok_or_else(|| anyhow::anyhow!("Target block hash {new_tip_block_hash:#x} not found"))?;
        let target_block_info = self
            .inner
            .get_block_info(target_block_n)
            .context("Getting target block info")?
            .ok_or_else(|| anyhow::anyhow!("Target block info not found for block_n={target_block_n}"))?;

        let current_head_projection = self.inner.get_head_projection()?;
        let (current_tip, had_preconfirmed_tip) = match current_head_projection {
            StorageHeadProjection::Empty => bail!("Cannot revert when chain is empty"),
            StorageHeadProjection::Confirmed(block_n) => (block_n, false),
            StorageHeadProjection::Preconfirmed { header, .. } => (
                header
                    .block_number
                    .checked_sub(1)
                    .ok_or_else(|| anyhow::anyhow!("Preconfirmed block is at genesis"))?,
                true,
            ),
        };
        let current_tip_info = self
            .inner
            .get_block_info(current_tip)
            .context("Getting current tip block info")?
            .ok_or_else(|| anyhow::anyhow!("Current tip block info not found"))?;

        Ok(ReorgContext { target_block_n, target_block_info, current_tip, current_tip_info, had_preconfirmed_tip })
    }

    /// Handles a no-distance reorg, including removal of an external preconfirmed tip.
    ///
    /// Returns a result only when the request is complete; otherwise the caller continues the full reorg.
    fn finish_same_tip_reorg(&self, context: &ReorgContext, new_tip_block_hash: &Felt) -> Result<Option<(u64, Felt)>> {
        if context.target_block_n != context.current_tip {
            return Ok(None);
        }

        if context.had_preconfirmed_tip {
            tracing::info!(
                "🔄 REORG: Clearing preconfirmed tip while keeping confirmed head at block_n={}",
                context.target_block_n
            );
            self.replace_head_projection(&StorageHeadProjection::Confirmed(context.target_block_n))
                .context("Clearing preconfirmed head projection during revert")?;
            self.flush().context("Flushing database after clearing preconfirmed tip")?;
        } else {
            tracing::info!("🔄 REORG: Already at common ancestor block_n={}, no revert needed", context.target_block_n);
        }
        Ok(Some((context.target_block_n, *new_tip_block_hash)))
    }

    /// Preflights all L1-message cleanup and derives the replay cursor for the reverted range.
    ///
    /// Missing source-block metadata is rejected before trie or block data is mutated.
    fn prepare_l1_rewind(&self, target_block_n: u64, current_tip: u64) -> Result<L1RewindPlan> {
        let reverted_nonces = self
            .inner
            .collect_reverted_l1_handler_nonces(target_block_n, current_tip)
            .context("Collecting reverted L1 handler nonces")?;
        let pending_nonces =
            self.inner.get_all_pending_message_nonces().context("Collecting pending L1 message nonces")?;

        let mut nonces_to_cleanup = Vec::with_capacity(reverted_nonces.len() + pending_nonces.len());
        nonces_to_cleanup.extend(reverted_nonces.iter().copied());
        nonces_to_cleanup.extend(pending_nonces.iter().copied());
        nonces_to_cleanup.sort_unstable();
        nonces_to_cleanup.dedup();

        let mut min_source_l1_block: Option<u64> = None;
        let mut missing_source_block_nonces = Vec::new();
        for nonce in nonces_to_cleanup.iter().copied() {
            match self
                .inner
                .get_l1_handler_l1_block_by_nonce(nonce)
                .with_context(|| format!("Fetching L1 handler L1 block for cleanup nonce={nonce}"))?
            {
                Some(l1_block_n) => {
                    min_source_l1_block =
                        Some(min_source_l1_block.map_or(l1_block_n, |current| current.min(l1_block_n)));
                }
                None => missing_source_block_nonces.push(nonce),
            }
        }
        missing_source_block_nonces.sort_unstable();
        if !missing_source_block_nonces.is_empty() {
            let sample: Vec<u64> = missing_source_block_nonces.iter().copied().take(8).collect();
            bail!(
                "Cannot revert: missing L1 handler L1 block mapping for {} L1 message nonce(s) scheduled for cleanup (sample={sample:?}).",
                missing_source_block_nonces.len()
            );
        }

        let sync_tip_after_revert = min_source_l1_block.map(|block_n| block_n.saturating_sub(1));
        tracing::info!(
            "🔁 REORG preflight: reverted_l1_handler_nonces={}, pending_l1_message_nonces={}, l1_message_cleanup_nonces={}, min_source_l1_block={:?}, next_l1_sync_tip={:?}",
            reverted_nonces.len(),
            pending_nonces.len(),
            nonces_to_cleanup.len(),
            min_source_l1_block,
            sync_tip_after_revert
        );
        Ok(L1RewindPlan { nonces_to_cleanup, rewind_from_l1_block: min_source_l1_block, sync_tip_after_revert })
    }

    /// Reverts and commits all three materialized tries to one revision.
    ///
    /// Each trie starts from its own persisted head so interrupted prior reverts remain idempotent.
    fn revert_and_commit_tries(&self, heads: TrieLogHeads, target_block_n: u64, checkpoint_floor: bool) -> Result<()> {
        let target_id = BasicId::new(target_block_n);
        let target_label = if checkpoint_floor { "checkpoint floor" } else { "target" };
        tracing::debug!("🌳 REORG: Reverting contract trie to {target_label}...");
        let mut contract_trie = self.contract_trie_for_revert();
        let contract_needs_commit = revert_single_trie("contract", &mut contract_trie, heads.contract, target_block_n)?;

        tracing::debug!("🌳 REORG: Reverting contract storage trie to {target_label}...");
        let mut storage_trie = self.contract_storage_trie_for_revert();
        let storage_needs_commit =
            revert_single_trie("contract storage", &mut storage_trie, heads.contract_storage, target_block_n)?;

        tracing::debug!("🌳 REORG: Reverting class trie to {target_label}...");
        let mut class_trie = self.class_trie_for_revert();
        let class_needs_commit = revert_single_trie("class", &mut class_trie, heads.class, target_block_n)?;

        tracing::info!("💾 REORG: Committing tries after revert to {target_label}...");
        if contract_needs_commit {
            contract_trie
                .commit(target_id)
                .map_err(|error| anyhow::anyhow!("Failed to commit contract trie after revert: {error:?}"))?;
        }
        if storage_needs_commit {
            storage_trie
                .commit(target_id)
                .map_err(|error| anyhow::anyhow!("Failed to commit contract storage trie after revert: {error:?}"))?;
        }
        if class_needs_commit {
            class_trie
                .commit(target_id)
                .map_err(|error| anyhow::anyhow!("Failed to commit class trie after revert: {error:?}"))?;
        }
        Ok(())
    }

    /// Rebuilds the target root from the retained checkpoint floor and ordered state diffs.
    ///
    /// Checkpoint metadata is rewound before replay so it never points at a pruned future revision.
    fn revert_from_checkpoint_floor(
        &self,
        context: &ReorgContext,
        heads: TrieLogHeads,
        checkpoint_ceiling: u64,
    ) -> Result<()> {
        let checkpoint_floor = self
            .inner
            .get_parallel_merkle_checkpoint_floor(context.target_block_n)
            .context("Reading parallel merkle checkpoint floor before revert")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing parallel merkle checkpoint floor for revert target {} with latest checkpoint {checkpoint_ceiling}",
                    context.target_block_n
                )
            })?;
        ensure_parallel_merkle_revert_is_retained(
            checkpoint_ceiling,
            context.target_block_n,
            checkpoint_floor,
            self.inner.config.max_saved_trie_logs,
        )?;
        tracing::info!(
            "🌳 REORG: Floor-revert mode with checkpoints (floor={}, ceiling={}, target={})",
            checkpoint_floor,
            checkpoint_ceiling,
            context.target_block_n
        );

        self.revert_and_commit_tries(heads, checkpoint_floor, true)?;
        self.inner
            .remove_parallel_merkle_checkpoints_above(context.target_block_n)
            .context("Rewinding parallel merkle checkpoint metadata after floor revert")?;

        if context.target_block_n > checkpoint_floor {
            self.replay_state_diffs_inclusive(
                checkpoint_floor + 1,
                context.target_block_n,
                context.target_block_info.header.protocol_version,
                "checkpoint-floor reorg",
            )
            .context("Replaying ordered state diffs after floor revert")?;
            self.inner
                .write_parallel_merkle_checkpoint(context.target_block_n)
                .context("Marking replay target as checkpoint after floor revert")?;
        } else {
            tracing::info!("🌳 REORG: Target block is checkpoint floor; no cumulative replay needed");
        }
        Ok(())
    }

    /// Selects checkpoint-floor or direct trie rollback from the durable checkpoint metadata.
    ///
    /// Divergent trie heads are expected after interrupted recovery and are handled independently.
    fn revert_tries(&self, context: &ReorgContext) -> Result<()> {
        let heads = self.trie_log_heads().context("Reading bonsai trie log heads before reorg")?;
        let latest_applied_trie_update = self.get_latest_applied_trie_update().ok().flatten();
        tracing::info!(
            "🌳 REORG: Reverting bonsai tries from current={} to target={}",
            context.current_tip,
            context.target_block_n
        );
        tracing::info!(
            "🌳 REORG: Trie log heads before revert: contract={:?}, contract_storage={:?}, class={:?}, latest_applied_trie_update={:?}",
            heads.contract,
            heads.contract_storage,
            heads.class,
            latest_applied_trie_update
        );
        if let Some(highest_head) = heads.highest().filter(|head| *head != context.current_tip) {
            tracing::warn!(
                "🌳 REORG: Confirmed chain tip ({}) diverges from latest persisted trie log head ({}). Reverting each trie from its actual head.",
                context.current_tip,
                highest_head
            );
        }

        match self
            .inner
            .get_parallel_merkle_latest_checkpoint()
            .context("Reading latest parallel merkle checkpoint before revert")?
        {
            Some(checkpoint_ceiling) => self.revert_from_checkpoint_floor(context, heads, checkpoint_ceiling),
            None => {
                self.revert_and_commit_tries(heads, context.target_block_n, false)?;
                tracing::info!("✅ REORG: All tries committed successfully");
                Ok(())
            }
        }
    }

    /// Verifies that the materialized trie root exactly matches the requested target block.
    ///
    /// The canonical head remains unchanged if verification fails.
    fn verify_reorg_target_root(&self, context: &ReorgContext) -> Result<()> {
        let expected_root = context.target_block_info.header.global_state_root;
        let actual_root = self
            .get_state_root_hash_at_version(context.target_block_info.header.protocol_version)
            .context("Reading global state root after trie revert")?;
        let matches = actual_root == expected_root;
        tracing::info!(
            "reorg_target_state_root_verification target_block_n={} expected_root={:#x} actual_root={:#x} match={}",
            context.target_block_n,
            expected_root,
            actual_root,
            matches
        );
        if !matches {
            tracing::error!(
                "reorg_target_state_root_mismatch target_block_n={} expected_root={:#x} actual_root={:#x}",
                context.target_block_n,
                expected_root,
                actual_root
            );
        }
        ensure_reorg_target_root_matches(context.target_block_n, expected_root, actual_root)
    }

    /// Publishes the reorg head, deletes its noncanonical suffix, rewinds snapshots, and flushes.
    ///
    /// The atomic head commit is the linearization point; suffix cleanup is safely repeatable on restart.
    fn commit_reorg(&self, context: &ReorgContext, l1_plan: &L1RewindPlan) -> Result<()> {
        tracing::info!(
            "🔗 REORG: Atomically publishing head block_n={} with {} L1 cleanup entries",
            context.target_block_n,
            l1_plan.nonces_to_cleanup.len()
        );
        self.commit_reorg_head(context.target_block_n, &l1_plan.nonces_to_cleanup, l1_plan.sync_tip_after_revert)?;
        tracing::info!("✅ REORG: Canonical head and recovery metadata committed successfully");

        let suffix_start = context.target_block_n.checked_add(1).context("Computing reorg suffix start")?;
        tracing::info!("📦 REORG: Removing noncanonical block suffix starting at block_n={suffix_start}");
        self.inner
            .remove_all_blocks_starting_from(suffix_start)
            .context("Removing noncanonical block suffix after reorg head commit")?;
        tracing::info!("✅ REORG: Noncanonical block suffix removed successfully");

        tracing::info!("📸 REORG: Updating snapshots to new head block_n={}", context.target_block_n);
        self.snapshots.rewind_to(context.target_block_n);
        if self.has_parallel_merkle_checkpoint(context.target_block_n)? {
            self.snapshots.pin_head(context.target_block_n);
        }
        tracing::info!("✅ REORG: Snapshots updated successfully");

        if let Some(l1_sync_tip) = l1_plan.sync_tip_after_revert {
            tracing::info!(
                "🔁 REORG: L1 messaging sync tip committed at block_n={l1_sync_tip} (from source block {:?})",
                l1_plan.rewind_from_l1_block
            );
        } else {
            tracing::info!("🔁 REORG: No L1 messaging rewind needed");
        }

        tracing::info!("💾 REORG: Flushing database to persist changes...");
        self.flush().context("Flushing database after reorg")?;
        tracing::info!("✅ REORG: Database flushed successfully");
        Ok(())
    }
}

/// Executes the complete crash-consistent reorg around a single atomic head commit.
///
/// Validation and L1 preflight precede mutation; suffix cleanup and flush follow root verification.
pub(super) fn execute_reorg(storage: &RocksDBStorage, new_tip_block_hash: &Felt) -> Result<(u64, Felt)> {
    tracing::info!("Reverting blockchain to block_hash={new_tip_block_hash:#x}");
    let context = storage.prepare_reorg(new_tip_block_hash)?;
    if let Some(result) = storage.finish_same_tip_reorg(&context, new_tip_block_hash)? {
        return Ok(result);
    }
    ensure!(
        context.target_block_n < context.current_tip,
        "Cannot revert to block_n={} which is > current tip={}",
        context.target_block_n,
        context.current_tip
    );

    let l1_plan = storage.prepare_l1_rewind(context.target_block_n, context.current_tip)?;
    tracing::info!(
        "🔄 REORG: Starting blockchain reorganization from block_n={} to block_n={}",
        context.current_tip,
        context.target_block_n
    );
    tracing::info!(
        "🔄 REORG: Target block hash={:#x}, current tip hash={:#x}",
        context.target_block_info.block_hash,
        context.current_tip_info.block_hash
    );

    storage.revert_tries(&context)?;
    storage.verify_reorg_target_root(&context)?;
    storage.commit_reorg(&context, &l1_plan)?;
    tracing::info!(
        "🎉 REORG: Blockchain reorganization completed successfully! Reverted to block_n={} block_hash={:#x}",
        context.target_block_n,
        context.target_block_info.block_hash
    );
    Ok((context.target_block_n, context.target_block_info.block_hash))
}
