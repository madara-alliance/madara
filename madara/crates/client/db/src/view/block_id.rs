// use crate::prelude::*;

// #[derive(thiserror::Error, Debug)]
// pub enum BlockResolutionError {
//     #[error("Tried to resolve the latest confirmed block but there is currently none")]
//     NoBlocks,
//     #[error("No block with this hash was found")]
//     BlockHashNotFound,
//     #[error("No block with this number was found")]
//     BlockNumberNotFound,
//     #[error(transparent)]
//     Internal(#[from] anyhow::Error),
// }

// pub trait BlockViewResolvable: Sized {
//     type Error;
//     fn resolve_block_view<D: MadaraStorageRead>(
//         &self,
//         backend: &Arc<MadaraBackend<D>>,
//     ) -> Result<MadaraBlockView<D>, Self::Error>;
// }

// impl<T: BlockViewResolvable> BlockViewResolvable for &T {
//     type Error = T::Error;
//     fn resolve_block_view<D: MadaraStorageRead>(
//         &self,
//         backend: &Arc<MadaraBackend<D>>,
//     ) -> Result<MadaraBlockView<D>, Self::Error> {
//         (*self).resolve_block_view(backend)
//     }
// }

// pub trait StateViewResolvable: Sized {
//     type Error;
//     fn resolve_state_view<D: MadaraStorageRead>(
//         &self,
//         view: &MadaraStateView<D>,
//     ) -> Result<MadaraStateView<D>, Self::Error>;
// }

// impl<T: StateViewResolvable> StateViewResolvable for &T {
//     type Error = T::Error;
//     fn resolve_state_view<D: MadaraStorageRead>(
//         &self,
//         view: &MadaraStateView<D>,
//     ) -> Result<MadaraStateView<D>, Self::Error> {
//         (*self).resolve_state_view(view)
//     }
// }

// // v0.7/v0.8 rpc

// impl StateViewResolvable for mp_rpc::v0_7_1::BlockId {
//     type Error = BlockResolutionError;
//     fn resolve_state_view<D: MadaraStorageRead>(
//         &self,
//         view: &MadaraStateView<D>,
//     ) -> Result<MadaraStateView<D>, Self::Error> {
//         match self {
//             Self::Tag(mp_rpc::v0_7_1::BlockTag::Pending) => Ok(view.clone()), // Same view.
//             Self::Tag(mp_rpc::v0_7_1::BlockTag::Latest) => Ok(view.view_on_latest_confirmed()),
//             Self::Hash(hash) => {
//                 if let Some(block_n) = view.find_block_by_hash(hash)? {
//                     Ok(view.view_on_confirmed(block_n).with_context(|| {
//                         format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
//                     })?)
//                 } else {
//                     Err(BlockResolutionError::BlockHashNotFound)
//                 }
//             }
//             Self::Number(block_n) => view.view_on_confirmed(*block_n).ok_or(BlockResolutionError::BlockNumberNotFound),
//         }
//     }
// }

// impl BlockViewResolvable for mp_rpc::v0_7_1::BlockId {
//     type Error = BlockResolutionError;
//     fn resolve_block_view<D: MadaraStorageRead>(
//         &self,
//         backend: &Arc<MadaraBackend<D>>,
//     ) -> Result<MadaraBlockView<D>, Self::Error> {
//         match self {
//             Self::Tag(mp_rpc::v0_7_1::BlockTag::Pending) => Ok(backend.block_view_on_preconfirmed_or_fake()?.into()),
//             Self::Tag(mp_rpc::v0_7_1::BlockTag::Latest) => {
//                 backend.block_view_on_last_confirmed().map(|b| b.into()).ok_or(BlockResolutionError::NoBlocks)
//             }
//             Self::Hash(hash) => {
//                 if let Some(block_n) = backend.db.find_block_hash(hash)? {
//                     Ok(backend
//                         .block_view_on_confirmed(block_n)
//                         .with_context(|| {
//                             format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
//                         })?
//                         .into())
//                 } else {
//                     Err(BlockResolutionError::BlockHashNotFound)
//                 }
//             }
//             Self::Number(block_n) => backend
//                 .block_view_on_confirmed(*block_n)
//                 .map(Into::into)
//                 .ok_or(BlockResolutionError::BlockNumberNotFound),
//         }
//     }
// }

// // v0.9 rpc

// impl StateViewResolvable for mp_rpc::v0_9_0::BlockId {
//     type Error = BlockResolutionError;
//     fn resolve_state_view<D: MadaraStorageRead>(
//         &self,
//         view: &MadaraStateView<D>,
//     ) -> Result<MadaraStateView<D>, Self::Error> {
//         match self {
//             Self::Tag(mp_rpc::v0_9_0::BlockTag::PreConfirmed) => Ok(view.clone()), // Same view.
//             Self::Tag(mp_rpc::v0_9_0::BlockTag::Latest) => Ok(view.view_on_latest_confirmed()),
//             Self::Tag(mp_rpc::v0_9_0::BlockTag::L1Accepted) => view
//                 .latest_l1_confirmed_block_n()
//                 .and_then(|block_number| view.view_on_confirmed(block_number))
//                 .map(|b| b.into())
//                 .ok_or(BlockResolutionError::NoBlocks),
//             Self::Hash(hash) => {
//                 if let Some(block_n) = view.find_block_by_hash(hash)? {
//                     Ok(view.view_on_confirmed(block_n).with_context(|| {
//                         format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
//                     })?)
//                 } else {
//                     Err(BlockResolutionError::BlockHashNotFound)
//                 }
//             }
//             Self::Number(block_n) => view.view_on_confirmed(*block_n).ok_or(BlockResolutionError::BlockNumberNotFound),
//         }
//     }
// }

// impl BlockViewResolvable for mp_rpc::v0_9_0::BlockId {
//     type Error = BlockResolutionError;
//     fn resolve_block_view<D: MadaraStorageRead>(
//         &self,
//         backend: &Arc<MadaraBackend<D>>,
//     ) -> Result<MadaraBlockView<D>, Self::Error> {
//         match self {
//             Self::Tag(mp_rpc::v0_9_0::BlockTag::PreConfirmed) => {
//                 Ok(backend.block_view_on_preconfirmed_or_fake()?.into())
//             }
//             Self::Tag(mp_rpc::v0_9_0::BlockTag::Latest) => {
//                 backend.block_view_on_last_confirmed().map(|b| b.into()).ok_or(BlockResolutionError::NoBlocks)
//             }
//             Self::Tag(mp_rpc::v0_9_0::BlockTag::L1Accepted) => backend
//                 .latest_l1_confirmed_block_n()
//                 .and_then(|block_number| backend.block_view_on_confirmed(block_number))
//                 .map(|b| b.into())
//                 .ok_or(BlockResolutionError::NoBlocks),
//             Self::Hash(hash) => {
//                 if let Some(block_n) = backend.db.find_block_hash(hash)? {
//                     Ok(backend
//                         .block_view_on_confirmed(block_n)
//                         .with_context(|| {
//                             format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
//                         })?
//                         .into())
//                 } else {
//                     Err(BlockResolutionError::BlockHashNotFound)
//                 }
//             }
//             Self::Number(block_n) => backend
//                 .block_view_on_confirmed(*block_n)
//                 .map(Into::into)
//                 .ok_or(BlockResolutionError::BlockNumberNotFound),
//         }
//     }
// }

// impl<D: MadaraStorageRead> MadaraStateView<D> {
//     /// Returns a state view on the latest confirmed block state. This view can be used to query the state from this block and earlier.
//     pub fn view_on<R: StateViewResolvable>(&self, block_id: R) -> Result<MadaraStateView<D>, R::Error> {
//         block_id.resolve_state_view(self)
//     }
// }

// impl<D: MadaraStorageRead> MadaraBackend<D> {
//     /// Returns a view on a block. This view is used to query content from that block.
//     ///
//     /// Note: When called using [`mp_rpc::BlockId`], this function may return a fake preconfirmed block if none was found.
//     pub fn block_view<R: BlockViewResolvable>(self: &Arc<Self>, block_id: R) -> Result<MadaraBlockView<D>, R::Error> {
//         block_id.resolve_block_view(self)
//     }

//     /// Returns a state view on the latest confirmed block state. This view can be used to query the state from this block and earlier.
//     pub fn view_on<R: StateViewResolvable>(self: &Arc<Self>, block_id: R) -> Result<MadaraStateView<D>, R::Error> {
//         block_id.resolve_state_view(&self.view_on_latest())
//     }
// }
