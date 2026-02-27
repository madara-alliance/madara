use crate::{storage::StorageChainTip, ChainTip};
use anyhow::{bail, ensure, Result};

/// Canonical in-memory chain head state used by internal services.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChainHeadState {
    /// Latest confirmed block number.
    pub confirmed_tip: Option<u64>,
    /// External preconfirmed tip (single tip projection for rpc/sync compatibility).
    pub external_preconfirmed_tip: Option<u64>,
    /// Internal preconfirmed tip used by block production internals.
    pub internal_preconfirmed_tip: Option<u64>,
}

impl ChainHeadState {
    /// Build chain head state from persisted storage tip.
    pub fn from_storage_tip(chain_tip: &StorageChainTip) -> Self {
        match chain_tip {
            StorageChainTip::Empty => Self::default(),
            StorageChainTip::Confirmed(block_n) => {
                Self { confirmed_tip: Some(*block_n), external_preconfirmed_tip: None, internal_preconfirmed_tip: None }
            }
            StorageChainTip::Preconfirmed { header, .. } => {
                let block_n = header.block_number;
                Self {
                    confirmed_tip: block_n.checked_sub(1),
                    external_preconfirmed_tip: Some(block_n),
                    internal_preconfirmed_tip: Some(block_n),
                }
            }
        }
    }

    /// Computes the next chain head state from an incoming tip transition.
    /// The transition must preserve the chain invariants.
    pub fn next_from_transition(self, new_tip: &ChainTip) -> Result<Self> {
        match new_tip {
            ChainTip::Empty => bail!("Cannot replace the chain tip to empty."),
            ChainTip::Confirmed(new_block_n) => {
                if let Some(current_preconfirmed) = self.external_preconfirmed_tip {
                    let new_plus_one = new_block_n.checked_add(1);
                    ensure!(
                        current_preconfirmed == *new_block_n || new_plus_one == Some(current_preconfirmed),
                        "Replacing chain head with confirmed requires preconfirmed tip to match new block or new block + 1. [current_head={self:?}, new_tip={new_tip:?}]"
                    );
                } else {
                    let expected_next = self.confirmed_tip.map(|n| n + 1).unwrap_or(/* genesis */ 0);
                    ensure!(
                        expected_next == *new_block_n,
                        "Replacing chain head from confirmed to confirmed requires the new block number to be one plus current confirmed. [current_head={self:?}, new_tip={new_tip:?}]"
                    );
                }

                Ok(Self {
                    confirmed_tip: Some(*new_block_n),
                    external_preconfirmed_tip: None,
                    internal_preconfirmed_tip: None,
                })
            }
            ChainTip::Preconfirmed(preconfirmed) => {
                let preconfirmed_block_n = preconfirmed.header.block_number;
                if let Some(current_preconfirmed) = self.external_preconfirmed_tip {
                    ensure!(
                        current_preconfirmed == preconfirmed_block_n,
                        "Replacing chain head from preconfirmed to preconfirmed requires matching block number. [current_head={self:?}, new_tip={new_tip:?}]"
                    );
                } else {
                    let expected_preconfirmed = self.confirmed_tip.map(|n| n + 1).unwrap_or(/* genesis */ 0);
                    ensure!(
                        expected_preconfirmed == preconfirmed_block_n,
                        "Replacing chain head from confirmed to preconfirmed requires new block number to be current confirmed + 1. [current_head={self:?}, new_tip={new_tip:?}]"
                    );
                }

                Ok(Self {
                    confirmed_tip: preconfirmed_block_n.checked_sub(1),
                    external_preconfirmed_tip: Some(preconfirmed_block_n),
                    internal_preconfirmed_tip: Some(preconfirmed_block_n),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preconfirmed::PreconfirmedBlock;
    use mp_block::header::PreconfirmedHeader;
    use std::sync::Arc;

    fn mk_preconfirmed_tip(block_n: u64) -> ChainTip {
        ChainTip::Preconfirmed(Arc::new(PreconfirmedBlock::new(PreconfirmedHeader {
            block_number: block_n,
            ..Default::default()
        })))
    }

    #[test]
    fn transition_confirmed_to_preconfirmed_sets_both_tips() {
        let current =
            ChainHeadState { confirmed_tip: Some(3), external_preconfirmed_tip: None, internal_preconfirmed_tip: None };

        let next = current.next_from_transition(&mk_preconfirmed_tip(4)).expect("transition should succeed");
        assert_eq!(
            next,
            ChainHeadState {
                confirmed_tip: Some(3),
                external_preconfirmed_tip: Some(4),
                internal_preconfirmed_tip: Some(4)
            }
        );
    }

    #[test]
    fn transition_preconfirmed_to_confirmed_clears_both_tips() {
        let current = ChainHeadState {
            confirmed_tip: Some(3),
            external_preconfirmed_tip: Some(4),
            internal_preconfirmed_tip: Some(4),
        };

        let next = current.next_from_transition(&ChainTip::Confirmed(4)).expect("transition should succeed");
        assert_eq!(
            next,
            ChainHeadState { confirmed_tip: Some(4), external_preconfirmed_tip: None, internal_preconfirmed_tip: None }
        );
    }
}
