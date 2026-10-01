//! Confirmed-trie repair starts at the newest root-verified checkpoint, independently of
//! each trie's last mutation. A durable intent marker survives partial rollback. Failed
//! candidates retain checkpoint metadata so another attempt can search backward again.
//!
//! Replay publishes one verified block's three trie overlays and checkpoint atomically.
//! Restart can therefore resume at the latest checkpoint even after replay prunes older logs.
//! Only a marked repair runs during DB open; ordinary full-node cursors remain independent.

use super::*;
use crate::rocksdb::{
    global_trie::in_memory::BoundaryFlushOutcome, meta::ConfirmedTrieRecovery, rocksdb_snapshot::SnapshotWithDBArc,
};

impl RocksDBStorage {
    /// Records intent and the original retention ceiling before the first trie can change.
    pub(super) fn begin_confirmed_trie_recovery(&self, confirmed_tip: Option<u64>) -> Result<ConfirmedTrieRecovery> {
        let recovery = match self.inner.get_confirmed_trie_recovery()? {
            Some(recovery) => {
                ensure!(
                    recovery.confirmed_tip == confirmed_tip,
                    "Confirmed head changed during unfinished trie recovery"
                );
                recovery
            }
            None => ConfirmedTrieRecovery {
                confirmed_tip,
                latest_trie_revision: self
                    .trie_log_heads()?
                    .highest()
                    .max(self.get_parallel_merkle_latest_checkpoint()?),
            },
        };
        self.inner.write_confirmed_trie_recovery(Some(recovery))?;
        self.flush().context("Persisting confirmed trie recovery intent")?;
        Ok(recovery)
    }

    /// Flushes repaired tries and metadata before removing durable recovery intent.
    pub(super) fn finish_confirmed_trie_recovery(&self) -> Result<()> {
        if self.inner.get_confirmed_trie_recovery()?.is_some() {
            self.flush().context("Persisting repaired confirmed trie state")?;
            // Keep nested repair intent until the outer reorg can clear both markers
            // atomically. Its original floor may already be outside retained history.
            if self.inner.get_reorg_recovery_floor()?.is_some() {
                return Ok(());
            }
            self.inner.write_confirmed_trie_recovery(None)?;
            self.flush().context("Persisting confirmed trie recovery completion")?;
        }
        Ok(())
    }

    /// Tries checkpoints newest first. Only root mismatches permit fallback; I/O and missing
    /// headers remain errors. Exhausting existing checkpoints never silently rebuilds genesis.
    pub(super) fn select_verified_recovery_floor(
        &self,
        confirmed_tip: u64,
        recovery: ConfirmedTrieRecovery,
        context: &str,
    ) -> Result<(Option<u64>, bool)> {
        let mut candidate = self.get_parallel_merkle_checkpoint_floor(confirmed_tip)?;
        let mut rolled_back = false;
        loop {
            let expected_root = self.checkpoint_floor_root(candidate)?;
            if let Some(floor) = candidate {
                if let Some(current) = self.trie_log_heads()?.highest().filter(|current| *current > floor) {
                    ensure_parallel_merkle_revert_is_retained(
                        recovery.latest_trie_revision.unwrap_or(current).max(current),
                        confirmed_tip,
                        floor,
                        self.inner.config.max_saved_trie_logs,
                    )?;
                }
            }
            rolled_back |= self.rollback_tries_to_checkpoint_floor(candidate, context)?;
            let actual_root = match candidate {
                Some(floor) => {
                    let info = self.confirmed_block_for_reconcile(floor)?;
                    self.get_state_root_hash_at_version(info.header.protocol_version)?
                }
                None => self.get_state_root_hash()?,
            };
            if actual_root == expected_root {
                self.rewind_parallel_merkle_checkpoints(candidate)?;
                tracing::info!(
                    context, confirmed_tip, checkpoint_floor = ?candidate,
                    replay_blocks = candidate.map_or(confirmed_tip.saturating_add(1), |floor| confirmed_tip - floor),
                    "Selected verified confirmed trie recovery checkpoint"
                );
                return Ok((candidate, rolled_back));
            }
            tracing::warn!(
                context, confirmed_tip, checkpoint_floor = ?candidate, %expected_root, %actual_root,
                "Confirmed trie recovery checkpoint root mismatch; trying older checkpoint"
            );
            candidate = match candidate.and_then(|floor| floor.checked_sub(1)) {
                Some(ceiling) => self.get_parallel_merkle_checkpoint_floor(ceiling)?,
                None => None,
            };
            ensure!(
                candidate.is_some(),
                "No verified checkpoint remains for confirmed trie recovery at block {confirmed_tip}"
            );
        }
    }

    /// Replays in block order with a root check and atomic checkpoint for every block.
    /// Using private overlays avoids half-written serial commits and keeps retries within
    /// retained history, even when the original recovery floor is pruned during a long replay.
    pub(super) fn replay_confirmed_trie(&self, from_block_n: u64, confirmed_tip: u64) -> Result<()> {
        for block_n in from_block_n..=confirmed_tip {
            let info = self.confirmed_block_for_reconcile(block_n)?;
            let diff = self.get_block_state_diff(block_n)?.context("Missing confirmed recovery state diff")?;
            let base = block_n.checked_sub(1);
            let snapshot = Arc::new(SnapshotWithDBArc::new(Arc::clone(&self.inner)));
            let computed = self.compute_root_from_selected_snapshot(
                base,
                snapshot,
                block_n,
                &diff,
                info.header.protocol_version,
                true,
                false,
            )?;
            ensure!(
                computed.state_root == info.header.global_state_root,
                "Confirmed trie replay root mismatch at block {block_n}: expected {:#x}, got {:#x}",
                info.header.global_state_root,
                computed.state_root,
            );
            let overlay = computed.overlay.context("Missing confirmed recovery trie overlay")?;
            ensure!(
                self.flush_overlay_and_checkpoint(block_n, 1, base, &overlay)? == BoundaryFlushOutcome::Persisted,
                "Confirmed trie recovery overlay became stale at block {block_n}"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
