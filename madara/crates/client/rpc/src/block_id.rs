use crate::{Starknet, StarknetRpcApiError};
use anyhow::Context;
use mc_db::{MadaraBlockView, MadaraStateView, MadaraStorageRead};

pub trait BlockViewResolvable: Sized {
    fn resolve_block_view(&self, starknet: &Starknet) -> Result<MadaraBlockView, StarknetRpcApiError>;
}

pub trait StateViewResolvable: Sized {
    fn resolve_state_view(&self, starknet: &Starknet) -> Result<MadaraStateView, StarknetRpcApiError>;
}

// v0.7/v0.8 rpc

impl StateViewResolvable for mp_rpc::v0_7_1::BlockId {
    fn resolve_state_view(&self, starknet: &Starknet) -> Result<MadaraStateView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Pending) => {
                let mut view = starknet.backend.view_on_latest();
                if !starknet.show_preconfirmed_in_pre_v0_9_rpcs {
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

impl BlockViewResolvable for mp_rpc::v0_7_1::BlockId {
    fn resolve_block_view(&self, starknet: &Starknet) -> Result<MadaraBlockView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Pending) => {
                let mut view = starknet.backend.block_view_on_preconfirmed_or_fake()?;
                if !starknet.show_preconfirmed_in_pre_v0_9_rpcs {
                    view.trim_view_to_start() // None of the pre-confirmed transactions should be shown in the RPCs.
                }
                Ok(view.into())
            }
            Self::Tag(mp_rpc::v0_7_1::BlockTag::Latest) => {
                starknet.backend.block_view_on_last_confirmed().map(|b| b.into()).ok_or(StarknetRpcApiError::NoBlocks)
            }
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
                .map(|b| b.into())
                .ok_or(StarknetRpcApiError::NoBlocks),
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

impl BlockViewResolvable for mp_rpc::v0_9_0::BlockId {
    fn resolve_block_view(&self, starknet: &Starknet) -> Result<MadaraBlockView, StarknetRpcApiError> {
        match self {
            Self::Tag(mp_rpc::v0_9_0::BlockTag::PreConfirmed) => {
                Ok(starknet.backend.block_view_on_preconfirmed_or_fake()?.into())
            }
            Self::Tag(mp_rpc::v0_9_0::BlockTag::Latest) => {
                starknet.backend.block_view_on_last_confirmed().map(|b| b.into()).ok_or(StarknetRpcApiError::NoBlocks)
            }
            Self::Tag(mp_rpc::v0_9_0::BlockTag::L1Accepted) => starknet
                .backend
                .latest_l1_confirmed_block_n()
                .and_then(|block_number| starknet.backend.block_view_on_confirmed(block_number))
                .map(|b| b.into())
                .ok_or(StarknetRpcApiError::NoBlocks),
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
}
