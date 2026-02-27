use crate::ChainTip;

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
    /// Derive chain head state from the current chain tip.
    pub fn from_chain_tip(chain_tip: &ChainTip) -> Self {
        match chain_tip {
            ChainTip::Empty => Self::default(),
            ChainTip::Confirmed(block_n) => {
                Self { confirmed_tip: Some(*block_n), external_preconfirmed_tip: None, internal_preconfirmed_tip: None }
            }
            ChainTip::Preconfirmed(preconfirmed) => {
                let block_n = preconfirmed.header.block_number;
                Self {
                    confirmed_tip: block_n.checked_sub(1),
                    external_preconfirmed_tip: Some(block_n),
                    internal_preconfirmed_tip: Some(block_n),
                }
            }
        }
    }
}
