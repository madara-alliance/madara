use crate::storage::StorageHeadProjection;
use anyhow::{ensure, Context, Result};

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
    /// Build chain head state from persisted head projection.
    pub fn from_head_projection(head_projection: &StorageHeadProjection) -> Self {
        match head_projection {
            StorageHeadProjection::Empty => Self::default(),
            StorageHeadProjection::Confirmed(block_n) => {
                Self { confirmed_tip: Some(*block_n), external_preconfirmed_tip: None, internal_preconfirmed_tip: None }
            }
            StorageHeadProjection::Preconfirmed { header, .. } => {
                let block_n = header.block_number;
                Self {
                    confirmed_tip: block_n.checked_sub(1),
                    external_preconfirmed_tip: Some(block_n),
                    internal_preconfirmed_tip: Some(block_n),
                }
            }
        }
    }

    /// Validate cross-field invariants on the chain head state.
    ///
    /// Invariants enforced:
    /// 1. When external preconfirmed tip exists, it must be exactly confirmed_tip + 1.
    /// 2. internal_preconfirmed_tip must never be behind external_preconfirmed_tip.
    /// 3. When no external preconfirmed tip exists, internal must also be None.
    pub fn validate_cross_field_invariants(&self) -> Result<()> {
        if let Some(external) = self.external_preconfirmed_tip {
            let expected_confirmed = external.checked_sub(1);
            ensure!(
                self.confirmed_tip == expected_confirmed,
                "external_preconfirmed_tip ({external}) must be exactly confirmed_tip + 1 (expected confirmed_tip={expected_confirmed:?}, got {:?}). [head={self:?}]",
                self.confirmed_tip
            );
            if let Some(internal) = self.internal_preconfirmed_tip {
                ensure!(
                    internal >= external,
                    "internal_preconfirmed_tip ({internal}) must not be behind external_preconfirmed_tip ({external}). [head={self:?}]"
                );
            } else {
                anyhow::bail!(
                    "internal_preconfirmed_tip is None while external_preconfirmed_tip is Some({external}). [head={self:?}]"
                );
            }
        } else {
            ensure!(
                self.internal_preconfirmed_tip.is_none(),
                "internal_preconfirmed_tip ({:?}) must be None when external_preconfirmed_tip is None. [head={self:?}]",
                self.internal_preconfirmed_tip
            );
        }
        Ok(())
    }

    /// Compute the next chain head state when the new head is confirmed.
    pub fn next_for_confirmed(self, new_block_n: u64) -> Result<Self> {
        if let Some(current_preconfirmed) = self.external_preconfirmed_tip {
            ensure!(
                current_preconfirmed == new_block_n,
                "Replacing chain head with confirmed requires external preconfirmed tip to match new block. [current_head={self:?}, new_confirmed={new_block_n}]"
            );

            let internal_preconfirmed = self
                .internal_preconfirmed_tip
                .context("internal_preconfirmed_tip must be present when external_preconfirmed_tip is present")?;
            if internal_preconfirmed > new_block_n {
                let next_external = new_block_n
                    .checked_add(1)
                    .context("block number overflow while promoting external preconfirmed tip")?;
                ensure!(
                    next_external <= internal_preconfirmed,
                    "next external preconfirmed tip ({next_external}) must be <= internal_preconfirmed_tip ({internal_preconfirmed}). [current_head={self:?}, new_confirmed={new_block_n}]"
                );
                return Ok(Self {
                    confirmed_tip: Some(new_block_n),
                    external_preconfirmed_tip: Some(next_external),
                    internal_preconfirmed_tip: Some(internal_preconfirmed),
                });
            }

            ensure!(
                internal_preconfirmed == new_block_n,
                "internal_preconfirmed_tip ({internal_preconfirmed}) must equal confirmed block ({new_block_n}) when no runahead remains. [current_head={self:?}]"
            );
            return Ok(Self {
                confirmed_tip: Some(new_block_n),
                external_preconfirmed_tip: None,
                internal_preconfirmed_tip: None,
            });
        } else {
            let expected_next = self
                .confirmed_tip
                .map(|n| n.checked_add(1).context("block number overflow while computing next confirmed tip"))
                .transpose()?
                .unwrap_or(/* genesis */ 0);
            ensure!(
                expected_next == new_block_n,
                "Replacing chain head from confirmed to confirmed requires the new block number to be one plus current confirmed. [current_head={self:?}, new_confirmed={new_block_n}]"
            );
        }

        Ok(Self { confirmed_tip: Some(new_block_n), external_preconfirmed_tip: None, internal_preconfirmed_tip: None })
    }

    /// Compute the next chain head state when the new head is preconfirmed.
    pub fn next_for_preconfirmed(self, preconfirmed_block_n: u64) -> Result<Self> {
        if let Some(current_preconfirmed) = self.external_preconfirmed_tip {
            let internal_preconfirmed = self
                .internal_preconfirmed_tip
                .context("internal_preconfirmed_tip must be present when external_preconfirmed_tip is present")?;
            let expected_next = internal_preconfirmed
                .checked_add(1)
                .context("block number overflow while computing next internal preconfirmed tip")?;
            ensure!(
                expected_next == preconfirmed_block_n,
                "Replacing chain head from preconfirmed to preconfirmed requires new internal preconfirmed block to be exactly one plus current internal tip. [current_head={self:?}, new_preconfirmed={preconfirmed_block_n}]"
            );
            return Ok(Self {
                confirmed_tip: self.confirmed_tip,
                external_preconfirmed_tip: Some(current_preconfirmed),
                internal_preconfirmed_tip: Some(preconfirmed_block_n),
            });
        } else {
            let expected_preconfirmed = self
                .confirmed_tip
                .map(|n| n.checked_add(1).context("block number overflow while computing next preconfirmed tip"))
                .transpose()?
                .unwrap_or(/* genesis */ 0);
            ensure!(
                expected_preconfirmed == preconfirmed_block_n,
                "Replacing chain head from confirmed to preconfirmed requires new block number to be current confirmed + 1. [current_head={self:?}, new_preconfirmed={preconfirmed_block_n}]"
            );
        }

        Ok(Self {
            confirmed_tip: preconfirmed_block_n.checked_sub(1),
            external_preconfirmed_tip: Some(preconfirmed_block_n),
            internal_preconfirmed_tip: Some(preconfirmed_block_n),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Matrix: cross-field invariant validation.
    #[rstest]
    #[case::valid_no_preconfirmed(Some(5), None, None, true)]
    #[case::valid_genesis_empty(None, None, None, true)]
    #[case::valid_genesis_preconfirmed(None, Some(0), Some(0), true)]
    #[case::valid_with_preconfirmed(Some(4), Some(5), Some(5), true)]
    #[case::valid_internal_ahead(Some(4), Some(5), Some(6), true)]
    #[case::invalid_external_not_confirmed_plus_one(None, Some(1), Some(1), false)]
    #[case::invalid_confirmed_ge_external(Some(5), Some(5), Some(5), false)]
    #[case::invalid_confirmed_exceeds_external(Some(6), Some(5), Some(5), false)]
    #[case::invalid_internal_behind_external(Some(4), Some(5), Some(4), false)]
    #[case::invalid_internal_none_external_some(Some(4), Some(5), None, false)]
    #[case::invalid_internal_some_external_none(Some(4), None, Some(5), false)]
    fn cross_field_invariants(
        #[case] confirmed_tip: Option<u64>,
        #[case] external_preconfirmed_tip: Option<u64>,
        #[case] internal_preconfirmed_tip: Option<u64>,
        #[case] expect_ok: bool,
    ) {
        let state = ChainHeadState { confirmed_tip, external_preconfirmed_tip, internal_preconfirmed_tip };
        let result = state.validate_cross_field_invariants();
        assert_eq!(result.is_ok(), expect_ok, "unexpected invariant result for {state:?}: {result:?}");
    }

    /// Matrix: confirmed -> preconfirmed transitions (valid and invalid).
    #[rstest]
    #[case::valid_next(Some(3), None, None, 4, Some(3), Some(4), Some(4), true)]
    #[case::valid_genesis(None, None, None, 0, None, Some(0), Some(0), true)]
    #[case::valid_internal_runahead(Some(3), Some(4), Some(4), 5, Some(3), Some(4), Some(5), true)]
    #[case::invalid_gap(Some(3), None, None, 6, None, None, None, false)]
    #[case::invalid_behind(Some(3), None, None, 2, None, None, None, false)]
    #[case::invalid_reuse_same_preconfirmed(Some(3), Some(4), Some(4), 4, None, None, None, false)]
    #[case::invalid_skip_internal(Some(3), Some(4), Some(4), 6, None, None, None, false)]
    fn confirmed_to_preconfirmed(
        #[case] confirmed_tip: Option<u64>,
        #[case] external_preconfirmed_tip: Option<u64>,
        #[case] internal_preconfirmed_tip: Option<u64>,
        #[case] new_preconfirmed: u64,
        #[case] expected_confirmed_tip: Option<u64>,
        #[case] expected_external_tip: Option<u64>,
        #[case] expected_internal_tip: Option<u64>,
        #[case] expect_ok: bool,
    ) {
        let current = ChainHeadState { confirmed_tip, external_preconfirmed_tip, internal_preconfirmed_tip };
        let result = current.next_for_preconfirmed(new_preconfirmed);
        assert_eq!(result.is_ok(), expect_ok, "unexpected result for next_for_preconfirmed({new_preconfirmed})");
        if expect_ok {
            let next = result.unwrap();
            assert_eq!(next.confirmed_tip, expected_confirmed_tip);
            assert_eq!(next.external_preconfirmed_tip, expected_external_tip);
            assert_eq!(next.internal_preconfirmed_tip, expected_internal_tip);
        }
    }

    /// Matrix: preconfirmed -> confirmed transitions (valid and invalid).
    #[rstest]
    #[case::valid_close_without_runahead(Some(3), Some(4), Some(4), 4, Some(4), None, None, true)]
    #[case::valid_close_with_runahead(Some(3), Some(4), Some(6), 4, Some(4), Some(5), Some(6), true)]
    #[case::valid_confirmed_to_confirmed(Some(3), None, None, 4, Some(4), None, None, true)]
    #[case::valid_genesis(None, None, None, 0, Some(0), None, None, true)]
    #[case::invalid_gap(Some(3), None, None, 6, None, None, None, false)]
    #[case::invalid_behind(Some(3), None, None, 2, None, None, None, false)]
    #[case::invalid_mismatch_external(Some(3), Some(4), Some(6), 5, None, None, None, false)]
    #[case::invalid_confirm_previous(Some(3), Some(4), Some(4), 3, None, None, None, false)]
    fn preconfirmed_to_confirmed(
        #[case] confirmed_tip: Option<u64>,
        #[case] external_preconfirmed_tip: Option<u64>,
        #[case] internal_preconfirmed_tip: Option<u64>,
        #[case] new_confirmed: u64,
        #[case] expected_confirmed_tip: Option<u64>,
        #[case] expected_external_tip: Option<u64>,
        #[case] expected_internal_tip: Option<u64>,
        #[case] expect_ok: bool,
    ) {
        let current = ChainHeadState { confirmed_tip, external_preconfirmed_tip, internal_preconfirmed_tip };
        let result = current.next_for_confirmed(new_confirmed);
        assert_eq!(result.is_ok(), expect_ok, "unexpected result for next_for_confirmed({new_confirmed})");
        if expect_ok {
            let next = result.unwrap();
            assert_eq!(next.confirmed_tip, expected_confirmed_tip);
            assert_eq!(next.external_preconfirmed_tip, expected_external_tip);
            assert_eq!(next.internal_preconfirmed_tip, expected_internal_tip);
        }
    }
}
