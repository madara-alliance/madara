use crate::preconfirmed::PreconfirmedBlock;
use crate::prelude::*;
use crate::storage::StorageChainTip;
use crate::{MadaraBackend, MadaraStorage};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Current chain tip.
#[derive(Default, Clone)]
pub enum ChainTip {
    /// Empty pre-genesis state. There are no blocks currently in the backend.
    #[default]
    Empty,
    /// Latest block is a confirmed block.
    Confirmed(/* block_number */ u64),
    /// Latest block is a preconfirmed block.
    Preconfirmed(Arc<PreconfirmedBlock>),
}

// Use [`Arc::ptr_eq`] for quick equality check: we don't want to compare the content of the transactions
// for the preconfirmed block case.
impl PartialEq for ChainTip {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Confirmed(l0), Self::Confirmed(r0)) => l0 == r0,
            (Self::Preconfirmed(l0), Self::Preconfirmed(r0)) => Arc::ptr_eq(l0, r0),
            _ => false,
        }
    }
}
impl Eq for ChainTip {}

impl fmt::Debug for ChainTip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Empty"),
            Self::Confirmed(block_n) => write!(f, "Confirmed block_n={block_n}"),
            Self::Preconfirmed(preconfirmed_block) => {
                write!(f, "Preconfirmed block_n={}", preconfirmed_block.header.block_number)
            }
        }
    }
}

impl ChainTip {
    pub fn on_confirmed_block_n_or_empty(block_n: Option<u64>) -> Self {
        match block_n {
            Some(block_n) => Self::Confirmed(block_n),
            None => Self::Empty,
        }
    }

    /// Latest block_n, which may be the pre-confirmed block.
    pub fn block_n(&self) -> Option<u64> {
        match self {
            Self::Empty => None,
            Self::Confirmed(block_n) => Some(*block_n),
            Self::Preconfirmed(b) => Some(b.header.block_number),
        }
    }
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        match self {
            Self::Empty => None,
            Self::Preconfirmed(b) => b.header.block_number.checked_sub(1),
            Self::Confirmed(block_n) => Some(*block_n),
        }
    }
    pub fn is_preconfirmed(&self) -> bool {
        matches!(self, Self::Preconfirmed(_))
    }
    pub fn as_preconfirmed(&self) -> Option<&Arc<PreconfirmedBlock>> {
        match self {
            Self::Preconfirmed(b) => Some(b),
            _ => None,
        }
    }

    /// Convert to the chain tip type for use in the storage backend. It is distinct from our the internal
    /// ChainTip to hide implementation details from the storage implementation.
    pub(crate) fn to_storage(&self) -> StorageChainTip {
        match self {
            Self::Empty => StorageChainTip::Empty,
            Self::Confirmed(block_n) => StorageChainTip::Confirmed(*block_n),
            Self::Preconfirmed(preconfirmed_block) => StorageChainTip::Preconfirmed {
                header: preconfirmed_block.header.clone(),
                content: preconfirmed_block.content.borrow().executed_transactions().cloned().collect(),
            },
        }
    }
    pub(crate) fn from_storage(tip: StorageChainTip) -> Self {
        match tip {
            StorageChainTip::Empty => Self::Empty,
            StorageChainTip::Confirmed(block_n) => Self::Confirmed(block_n),
            StorageChainTip::Preconfirmed { header, content } => {
                Self::Preconfirmed(PreconfirmedBlock::new_with_content(header, content, /* candidates */ []).into())
            }
        }
    }
}

/// Canonical chain head view derived from persisted database/meta state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChainHeadState {
    /// Latest confirmed block persisted in `BLOCK_INFO_COLUMN`.
    pub confirmed_tip: Option<u64>,
    /// External preconfirmed tip exposed through `chain_tip` projection.
    pub external_preconfirmed_tip: Option<u64>,
    /// Internal staged/preconfirmed tip derived from contiguous stage-1 markers.
    pub internal_preconfirmed_tip: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ChainHeadRuntimeState {
    pub(super) head_state: ChainHeadState,
    // Stage-1 visible staged blocks. Kept bounded to confirmed+10 horizon.
    pub(super) staged_stage1_blocks: BTreeSet<u64>,
}

#[derive(Debug)]
pub(super) struct ChainHeadCoordinator {
    runtime: Mutex<ChainHeadRuntimeState>,
    head_state_tx: tokio::sync::watch::Sender<ChainHeadState>,
    chain_tip_tx: tokio::sync::watch::Sender<ChainTip>,
}

impl Default for ChainHeadCoordinator {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(ChainHeadRuntimeState::default()),
            head_state_tx: tokio::sync::watch::Sender::new(ChainHeadState::default()),
            chain_tip_tx: tokio::sync::watch::Sender::new(ChainTip::default()),
        }
    }
}

impl ChainHeadCoordinator {
    pub(super) fn with_runtime_mut<R>(&self, f: impl FnOnce(&mut ChainHeadRuntimeState) -> R) -> R {
        let mut runtime = self.runtime.lock().expect("Poisoned lock");
        f(&mut runtime)
    }

    pub(super) fn current_chain_tip(&self) -> ChainTip {
        self.chain_tip_tx.borrow().clone()
    }

    pub(super) fn current_head_state(&self) -> ChainHeadState {
        *self.head_state_tx.borrow()
    }

    pub(super) fn publish(&self, head_state: ChainHeadState, chain_tip: ChainTip) {
        self.head_state_tx.send_replace(head_state);
        self.chain_tip_tx.send_replace(chain_tip);
    }

    pub(super) fn watch_chain_tip(&self) -> tokio::sync::watch::Receiver<ChainTip> {
        self.chain_tip_tx.subscribe()
    }

    pub(super) fn watch_head_state(&self) -> tokio::sync::watch::Receiver<ChainHeadState> {
        self.head_state_tx.subscribe()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChainTipProjectionMode {
    RuntimeOnly,
    IncludeDbFallback,
}

impl<D: MadaraStorage> MadaraBackend<D> {
    fn compute_chain_head_state_from_db(&self) -> Result<ChainHeadState> {
        let confirmed_tip = self.db.get_latest_confirmed_block_n()?;
        let expected_external_preconfirmed = confirmed_tip.and_then(|n| n.checked_add(1)).unwrap_or(0);

        let external_preconfirmed_tip = match self.db.get_chain_tip()? {
            StorageChainTip::Preconfirmed { header, .. } => {
                if header.block_number == expected_external_preconfirmed {
                    Some(header.block_number)
                } else {
                    tracing::warn!(
                        confirmed_tip = ?confirmed_tip,
                        external_tip = header.block_number,
                        expected_external_preconfirmed,
                        "invalid external preconfirmed tip in DB metadata; ignoring it"
                    );
                    None
                }
            }
            StorageChainTip::Empty | StorageChainTip::Confirmed(_) => None,
        };

        let internal_from_block = expected_external_preconfirmed;
        let internal_to_block = confirmed_tip.and_then(|n| n.checked_add(10)).unwrap_or(10);
        let internal_preconfirmed_tip =
            self.db.get_parallel_merkle_staged_contiguous_tip(internal_from_block, internal_to_block)?;

        Ok(ChainHeadState { confirmed_tip, external_preconfirmed_tip, internal_preconfirmed_tip })
    }

    fn project_chain_tip_from_head_state(
        &self,
        head_state: ChainHeadState,
        preferred_preconfirmed: Option<Arc<PreconfirmedBlock>>,
        mode: ChainTipProjectionMode,
    ) -> Result<ChainTip> {
        let Some(external_preconfirmed_tip) = head_state.external_preconfirmed_tip else {
            return Ok(ChainTip::on_confirmed_block_n_or_empty(head_state.confirmed_tip));
        };

        if let Some(preferred) = preferred_preconfirmed {
            if preferred.header.block_number == external_preconfirmed_tip {
                return Ok(ChainTip::Preconfirmed(preferred));
            }
        }

        if let Some(current_preconfirmed) = self.chain_heads.current_chain_tip().as_preconfirmed().cloned() {
            if current_preconfirmed.header.block_number == external_preconfirmed_tip {
                return Ok(ChainTip::Preconfirmed(current_preconfirmed));
            }
        }

        if mode == ChainTipProjectionMode::IncludeDbFallback {
            if let StorageChainTip::Preconfirmed { header, content } = self.db.get_chain_tip()? {
                if header.block_number == external_preconfirmed_tip {
                    return Ok(ChainTip::from_storage(StorageChainTip::Preconfirmed { header, content }));
                }
            }
        }

        tracing::warn!(
            confirmed_tip = ?head_state.confirmed_tip,
            external_preconfirmed_tip,
            "failed to reconstruct external preconfirmed chain_tip content from DB; projecting confirmed tip instead"
        );
        Ok(ChainTip::on_confirmed_block_n_or_empty(head_state.confirmed_tip))
    }

    fn publish_head_state_and_projection(
        &self,
        head_state: ChainHeadState,
        preferred_preconfirmed: Option<Arc<PreconfirmedBlock>>,
        mode: ChainTipProjectionMode,
    ) -> Result<()> {
        let chain_tip = self.project_chain_tip_from_head_state(head_state, preferred_preconfirmed, mode)?;
        self.chain_heads.publish(head_state, chain_tip);
        Ok(())
    }

    fn recompute_internal_preconfirmed_tip(
        confirmed_tip: Option<u64>,
        staged_stage1_blocks: &BTreeSet<u64>,
    ) -> Option<u64> {
        let start = confirmed_tip.and_then(|n| n.checked_add(1)).unwrap_or(0);
        let end = confirmed_tip.and_then(|n| n.checked_add(10)).unwrap_or(10);
        let mut tip = None;
        for block_n in start..=end {
            if staged_stage1_blocks.contains(&block_n) {
                tip = Some(block_n);
            } else {
                break;
            }
        }
        tip
    }

    pub(super) fn head_on_new_preconfirmed_started(
        &self,
        _block_n: u64,
        preferred_preconfirmed: Arc<PreconfirmedBlock>,
    ) -> Result<()> {
        let new_preconfirmed_n = preferred_preconfirmed.header.block_number;
        let head_state = self.chain_heads.with_runtime_mut(|runtime| {
            runtime.head_state.external_preconfirmed_tip = Some(new_preconfirmed_n);
            runtime.head_state
        });
        self.publish_head_state_and_projection(
            head_state,
            Some(preferred_preconfirmed),
            ChainTipProjectionMode::RuntimeOnly,
        )?;
        Ok(())
    }

    pub(super) fn head_on_clear_preconfirmed(&self) -> Result<()> {
        let head_state = self.chain_heads.with_runtime_mut(|runtime| {
            runtime.head_state.external_preconfirmed_tip = None;
            runtime.head_state
        });
        self.publish_head_state_and_projection(head_state, None, ChainTipProjectionMode::RuntimeOnly)?;
        Ok(())
    }

    pub(super) fn head_on_stage1_staged(&self, block_n: u64) -> Result<()> {
        let head_state = self.chain_heads.with_runtime_mut(|runtime| {
            runtime.staged_stage1_blocks.insert(block_n);
            runtime.head_state.internal_preconfirmed_tip = Self::recompute_internal_preconfirmed_tip(
                runtime.head_state.confirmed_tip,
                &runtime.staged_stage1_blocks,
            );
            runtime.head_state
        });
        let preferred_preconfirmed = self.chain_heads.current_chain_tip().as_preconfirmed().cloned();
        self.publish_head_state_and_projection(
            head_state,
            preferred_preconfirmed,
            ChainTipProjectionMode::RuntimeOnly,
        )?;
        Ok(())
    }

    pub(super) fn head_on_confirmed(
        &self,
        block_n: u64,
        preferred_preconfirmed: Option<Arc<PreconfirmedBlock>>,
    ) -> Result<()> {
        let mut projection_preconfirmed =
            preferred_preconfirmed.clone().or_else(|| self.chain_heads.current_chain_tip().as_preconfirmed().cloned());
        let head_state = self.chain_heads.with_runtime_mut(|runtime| {
            if projection_preconfirmed.as_ref().is_some_and(|preconfirmed| preconfirmed.header.block_number <= block_n)
            {
                projection_preconfirmed = None;
            }
            runtime.head_state.confirmed_tip = Some(block_n);
            runtime.staged_stage1_blocks.retain(|staged_block_n| *staged_block_n > block_n);
            runtime.head_state.internal_preconfirmed_tip = Self::recompute_internal_preconfirmed_tip(
                runtime.head_state.confirmed_tip,
                &runtime.staged_stage1_blocks,
            );
            runtime.head_state.external_preconfirmed_tip = projection_preconfirmed.as_ref().and_then(|preconfirmed| {
                let expected = runtime.head_state.confirmed_tip.and_then(|n| n.checked_add(1)).unwrap_or(0);
                (preconfirmed.header.block_number == expected).then_some(expected)
            });
            runtime.head_state
        });
        self.publish_head_state_and_projection(
            head_state,
            projection_preconfirmed.clone(),
            ChainTipProjectionMode::RuntimeOnly,
        )?;
        Ok(())
    }

    pub(super) fn rebuild_chain_head_state_from_db_and_publish(
        &self,
        preferred_preconfirmed: Option<Arc<PreconfirmedBlock>>,
    ) -> Result<ChainHeadState> {
        let head_state = self.compute_chain_head_state_from_db()?;
        let mut staged_stage1_blocks = BTreeSet::new();
        let start = head_state.confirmed_tip.and_then(|n| n.checked_add(1)).unwrap_or(0);
        let end = head_state.confirmed_tip.and_then(|n| n.checked_add(10)).unwrap_or(10);
        for block_n in start..=end {
            if self.db.get_parallel_merkle_staged_block_header(block_n)?.is_some() {
                staged_stage1_blocks.insert(block_n);
            }
        }
        let reconstructed_head_state = ChainHeadState {
            internal_preconfirmed_tip: Self::recompute_internal_preconfirmed_tip(
                head_state.confirmed_tip,
                &staged_stage1_blocks,
            ),
            ..head_state
        };
        self.chain_heads.with_runtime_mut(|runtime| {
            runtime.head_state = reconstructed_head_state;
            runtime.staged_stage1_blocks = staged_stage1_blocks;
        });
        self.publish_head_state_and_projection(
            reconstructed_head_state,
            preferred_preconfirmed,
            ChainTipProjectionMode::IncludeDbFallback,
        )?;
        Ok(reconstructed_head_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_block::header::PreconfirmedHeader;
    use mp_chain_config::ChainConfig;

    fn make_preconfirmed(block_n: u64) -> Arc<PreconfirmedBlock> {
        Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: block_n, ..Default::default() }))
    }

    #[test]
    fn head_on_new_preconfirmed_started_updates_external_tip_and_projection() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let preconfirmed = make_preconfirmed(0);

        backend
            .head_on_new_preconfirmed_started(0, Arc::clone(&preconfirmed))
            .expect("new preconfirmed transition should succeed");

        let head_state = backend.chain_head_state();
        assert_eq!(head_state.confirmed_tip, None);
        assert_eq!(head_state.external_preconfirmed_tip, Some(0));
        assert_eq!(head_state.internal_preconfirmed_tip, None);
        assert!(matches!(
            backend.current_chain_tip(),
            ChainTip::Preconfirmed(block) if block.header.block_number == 0
        ));
    }

    #[test]
    fn runtime_projection_does_not_reuse_stale_preconfirmed_payload() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let stale = make_preconfirmed(12);
        backend.chain_heads.publish(
            ChainHeadState {
                confirmed_tip: Some(11),
                external_preconfirmed_tip: Some(12),
                internal_preconfirmed_tip: Some(12),
            },
            ChainTip::Preconfirmed(stale),
        );

        let projected = backend
            .project_chain_tip_from_head_state(
                ChainHeadState {
                    confirmed_tip: Some(12),
                    external_preconfirmed_tip: Some(13),
                    internal_preconfirmed_tip: Some(12),
                },
                None,
                ChainTipProjectionMode::RuntimeOnly,
            )
            .expect("projection should not fail");

        assert!(matches!(projected, ChainTip::Confirmed(12)));
    }
}
