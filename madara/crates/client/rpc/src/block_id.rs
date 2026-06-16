use crate::{Starknet, StarknetRpcApiError};
use anyhow::Context;
use mc_db::{MadaraBlockView, MadaraStateView, MadaraStorageRead};

pub trait BlockViewResolvable: Sized {
    fn resolve_block_view(&self, starknet: &Starknet) -> Result<MadaraBlockView, StarknetRpcApiError>;
}

pub trait StateViewResolvable: Sized {
    fn resolve_state_view(&self, starknet: &Starknet) -> Result<MadaraStateView, StarknetRpcApiError>;
}

pub trait EventRangeBoundResolvable: StateViewResolvable {
    fn event_range_number(&self) -> Option<u64>;
}

// v0.7/v0.8 rpc

impl StateViewResolvable for mp_rpc::v0_7_1::BlockId {
    fn resolve_state_view(&self, starknet: &Starknet) -> Result<MadaraStateView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Pending) => {
                let mut view = starknet.backend.view_on_latest();
                if !starknet.pre_v0_9_preconfirmed_as_pending {
                    view = view.view_on_latest_confirmed()
                }
                Ok(view)
            }
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Latest) => Ok(starknet.backend.view_on_latest_confirmed()),
            Self::Hash(hash) => {
                if let Some(block_n) = starknet.backend.view_on_latest().find_block_by_hash(hash)? {
                    Ok(starknet.backend.view_on_confirmed(block_n).with_context(|| {
                        format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
                    })?)
                } else {
                    Err(StarknetRpcApiError::BlockNotFound)
                }
            }
            Self::Number(block_n) => {
                starknet.backend.view_on_confirmed(*block_n).ok_or(StarknetRpcApiError::BlockNotFound)
            }
        }
    }
}

impl EventRangeBoundResolvable for mp_rpc::v0_7_1::BlockId {
    fn event_range_number(&self) -> Option<u64> {
        match self {
            Self::Number(block_n) => Some(*block_n),
            _ => None,
        }
    }
}

impl BlockViewResolvable for mp_rpc::v0_7_1::BlockId {
    fn resolve_block_view(&self, starknet: &Starknet) -> Result<MadaraBlockView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Pending) => {
                let mut view = starknet.backend.block_view_on_preconfirmed_or_fake()?;
                if !starknet.pre_v0_9_preconfirmed_as_pending {
                    view.trim_view_to_start() // None of the pre-confirmed transactions should be shown in the RPCs.
                }
                Ok(view.into())
            }
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Latest) => starknet
                .backend
                .block_view_on_last_confirmed()
                .map(|b| b.into())
                .ok_or(StarknetRpcApiError::BlockNotFound),
            Self::Hash(hash) => {
                if let Some(block_n) = starknet.backend.db.find_block_hash(hash)? {
                    Ok(starknet
                        .backend
                        .block_view_on_confirmed(block_n)
                        .with_context(|| {
                            format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
                        })?
                        .into())
                } else {
                    Err(StarknetRpcApiError::BlockNotFound)
                }
            }
            Self::Number(block_n) => starknet
                .backend
                .block_view_on_confirmed(*block_n)
                .map(Into::into)
                .ok_or(StarknetRpcApiError::BlockNotFound),
        }
    }
}

// v0.9 rpc

impl StateViewResolvable for mp_rpc::v0_9_0::BlockId {
    fn resolve_state_view(&self, starknet: &Starknet) -> Result<MadaraStateView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_9_0::BlockTag::PreConfirmed) => Ok(starknet.backend.view_on_latest()),
            Self::Tag(mp_rpc::v0_9_0::BlockTag::Latest) => Ok(starknet.backend.view_on_latest_confirmed()),
            Self::Tag(mp_rpc::v0_9_0::BlockTag::L1Accepted) => starknet
                .backend
                .latest_l1_confirmed_block_n()
                .and_then(|block_number| starknet.backend.view_on_confirmed(block_number))
                .ok_or(StarknetRpcApiError::BlockNotFound),
            Self::Hash(hash) => {
                if let Some(block_n) = starknet.backend.view_on_latest().find_block_by_hash(hash)? {
                    Ok(starknet.backend.view_on_confirmed(block_n).with_context(|| {
                        format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
                    })?)
                } else {
                    Err(StarknetRpcApiError::BlockNotFound)
                }
            }
            Self::Number(block_n) => {
                starknet.backend.view_on_confirmed(*block_n).ok_or(StarknetRpcApiError::BlockNotFound)
            }
        }
    }
}

impl EventRangeBoundResolvable for mp_rpc::v0_9_0::BlockId {
    fn event_range_number(&self) -> Option<u64> {
        match self {
            Self::Number(block_n) => Some(*block_n),
            _ => None,
        }
    }
}

impl BlockViewResolvable for mp_rpc::v0_9_0::BlockId {
    fn resolve_block_view(&self, starknet: &Starknet) -> Result<MadaraBlockView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_9_0::BlockTag::PreConfirmed) => {
                Ok(starknet.backend.block_view_on_preconfirmed_or_fake()?.into())
            }
            Self::Tag(mp_rpc::v0_9_0::BlockTag::Latest) => starknet
                .backend
                .block_view_on_last_confirmed()
                .map(|b| b.into())
                .ok_or(StarknetRpcApiError::BlockNotFound),
            Self::Tag(mp_rpc::v0_9_0::BlockTag::L1Accepted) => starknet
                .backend
                .latest_l1_confirmed_block_n()
                .and_then(|block_number| starknet.backend.block_view_on_confirmed(block_number))
                .map(|b| b.into())
                .ok_or(StarknetRpcApiError::BlockNotFound),
            Self::Hash(hash) => {
                if let Some(block_n) = starknet.backend.db.find_block_hash(hash)? {
                    Ok(starknet
                        .backend
                        .block_view_on_confirmed(block_n)
                        .with_context(|| {
                            format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
                        })?
                        .into())
                } else {
                    Err(StarknetRpcApiError::BlockNotFound)
                }
            }
            Self::Number(block_n) => starknet
                .backend
                .block_view_on_confirmed(*block_n)
                .map(Into::into)
                .ok_or(StarknetRpcApiError::BlockNotFound),
        }
    }
}

// v0.10 rpc

impl Starknet {
    pub fn resolve_block_view<R: BlockViewResolvable>(
        &self,
        block_id: R,
    ) -> Result<MadaraBlockView, StarknetRpcApiError> {
        block_id.resolve_block_view(self)
    }

    pub fn resolve_view_on<R: StateViewResolvable>(&self, block_id: R) -> Result<MadaraStateView, StarknetRpcApiError> {
        block_id.resolve_state_view(self)
    }

    /// Resolves a `getEvents` lower bound. Numeric bounds are raw block numbers and may point past
    /// the current tip. Hash/tag bounds must resolve to an actual block number; for example,
    /// `from_block = latest` on an empty chain is `BLOCK_NOT_FOUND`, matching Pathfinder.
    pub fn resolve_event_from_block_bound<R: EventRangeBoundResolvable>(
        &self,
        block_id: R,
    ) -> Result<u64, StarknetRpcApiError> {
        match block_id.event_range_number() {
            Some(block_n) => Ok(block_n),
            None => self.resolve_view_on(block_id)?.latest_block_n().ok_or(StarknetRpcApiError::BlockNotFound),
        }
    }

    /// Resolves a `getEvents` upper bound. Numeric bounds are raw block numbers and may point past
    /// the current tip. Hash/tag bounds resolve through the backend, but an empty-chain upper tag
    /// still scans up to block 0 so the range can collapse to an empty page.
    pub fn resolve_event_to_block_bound<R: EventRangeBoundResolvable>(
        &self,
        block_id: R,
    ) -> Result<u64, StarknetRpcApiError> {
        match block_id.event_range_number() {
            Some(block_n) => Ok(block_n),
            None => Ok(self.resolve_view_on(block_id)?.latest_block_n().unwrap_or(0)),
        }
    }
}
