use crate::{db::DBBackend, MadaraBackend, MadaraStorageError};
use mp_block::{header::PendingHeader, MadaraBlockInfo};
use mp_class::ConvertedClass;
use mp_receipt::TransactionReceipt;
use mp_state_update::TransactionStateUpdate;
use mp_transactions::Transaction;
use std::sync::{Arc, RwLock, RwLockReadGuard};

mod anchor;
mod query;

pub use anchor::*;

#[derive(Debug, Clone)]
pub struct PreconfirmedBlockTransaction {
    pub transaction: Transaction,
    pub receipt: TransactionReceipt,
    pub state_diff: TransactionStateUpdate,
    pub declared_class: Option<ConvertedClass>,
}

#[derive(Debug, Clone, Default)]
struct PreconfirmedBlockInner {
    txs: Vec<PreconfirmedBlockTransaction>,
}

#[derive(Debug)]
pub struct PreconfirmedBlock {
    pub block_n: u64,
    pub header: PendingHeader,
    content: RwLock<PreconfirmedBlockInner>,
}

pub(crate) struct PreconfirmedBlockInnerView<'a>(RwLockReadGuard<'a, PreconfirmedBlockInner>);
impl<'a> PreconfirmedBlockInnerView<'a> {
    pub fn transactions(&self) -> &[PreconfirmedBlockTransaction] {
        &self.0.txs
    }
}

impl PreconfirmedBlock {
    pub fn append(&self, txs: impl IntoIterator<Item = PreconfirmedBlockTransaction>) {
        let mut guard = self.content.write().expect("Poisoned lock");
        guard.txs.extend(txs);
    }
    pub fn n_txs(&self) -> u64 {
        self.content.read().expect("Poisoned lock").txs.len() as _
    }
    pub fn parent_block_n(&self) -> Option<u64> {
        self.block_n.checked_sub(1)
    }
    pub fn content(&self) -> PreconfirmedBlockInnerView {
        PreconfirmedBlockInnerView(self.content.read().expect("Poisoned lock"))
    }
}

pub enum PreconfirmedStatus {
    None { previous_block: Option<MadaraBlockInfo> },
    Preconfirmed(Arc<PreconfirmedBlock>),
}

impl PreconfirmedStatus {
    fn block_n(&self) -> u64 {
        match self {
            Self::None { previous_block } => {
                previous_block.as_ref().map(|b| b.header.block_number + 1).unwrap_or(/* genesis */ 0)
            }
            Self::Preconfirmed(preconfirmed_block) => preconfirmed_block.block_n,
        }
    }
}

#[derive(Clone)]
pub struct MadaraBackendBlockView<D: DBBackend> {
    pub(crate) backend: Arc<MadaraBackend<D>>,
    pub(crate) anchor: BlockAnchor,
}

impl<D: DBBackend> MadaraBackendBlockView<D> {
    fn new(backend: Arc<MadaraBackend<D>>, anchor: BlockAnchor) -> Self {
        Self { backend, anchor }
    }
}

#[derive(Clone)]
pub struct MadaraBackendView<D: DBBackend> {
    pub(crate) backend: Arc<MadaraBackend<D>>,
    pub(crate) anchor: Anchor,
}

impl<D: DBBackend> MadaraBackendView<D> {
    fn new(backend: Arc<MadaraBackend<D>>, anchor: Anchor) -> Self {
        Self { backend, anchor }
    }

    pub fn into_block_view_on(self, block_n: u64) -> Option<MadaraBackendBlockView<D>> {
        self.anchor
            .into_block_anchor()
            .filter(|anchor| block_n <= anchor.block_n())
            .map(|anchor| MadaraBackendBlockView::new(self.backend, anchor))
    }

    pub fn into_block_view(self) -> Option<MadaraBackendBlockView<D>> {
        self.anchor.into_block_anchor().map(|anchor| MadaraBackendBlockView::new(self.backend, anchor))
    }
}

impl<D: DBBackend> MadaraBackend<D> {
    fn view_on_anchor(self: &Arc<Self>, anchor: Anchor) -> MadaraBackendView<D> {
        MadaraBackendView::new(self.clone(), anchor)
    }
    fn block_view_on_anchor(self: &Arc<Self>, anchor: BlockAnchor) -> MadaraBackendBlockView<D> {
        MadaraBackendBlockView::new(self.clone(), anchor)
    }

    pub fn view_on(
        self: &Arc<Self>,
        anchor: impl IntoAnchor,
    ) -> Result<Option<MadaraBackendView<D>>, MadaraStorageError> {
        Ok(anchor.into_anchor(self.as_ref())?.map(|anchor| self.view_on_anchor(anchor)))
    }
    pub fn view_on_latest(self: &Arc<Self>) -> MadaraBackendView<D> {
        self.view_on_anchor(Anchor::new_on_block_n(self.get_latest_block_n_()))
    }
    pub fn view_on_preconfirmed(self: &Arc<Self>) -> MadaraBackendView<D> {
        self.view_on_anchor(Anchor::new_on_preconfirmed(self.get_preconfirmed().clone()))
    }

    pub fn block_view_on(
        self: &Arc<Self>,
        anchor: impl IntoAnchor,
    ) -> Result<Option<MadaraBackendBlockView<D>>, MadaraStorageError> {
        Ok(anchor.into_block_anchor(self.as_ref())?.map(|anchor| self.block_view_on_anchor(anchor)))
    }
    pub fn block_view_on_latest(self: &Arc<Self>) -> Option<MadaraBackendBlockView<D>> {
        Some(self.block_view_on_anchor(BlockAnchor::new_on_block_n(self.get_latest_block_n_()?)))
    }
    pub fn block_view_on_preconfirmed(self: &Arc<Self>) -> MadaraBackendBlockView<D> {
        self.block_view_on_anchor(BlockAnchor::new_on_preconfirmed(self.get_preconfirmed().clone()))
    }

    fn get_preconfirmed(&self) -> Arc<PreconfirmedBlock> {
        match self.preconfirmed.read().as_deref().expect("Poisoned lock") {
            PreconfirmedStatus::Preconfirmed(preconfirmed_block) => preconfirmed_block.clone(),
            PreconfirmedStatus::None { previous_block: _ } => {
                // Fake preconfirmed
                // Arc::new(PreconfirmedBlock {
                //     block_n: *new_block_n,
                //     header: PendingHeader {
                //         parent_block_hash: *parent_block_hash,
                //         sequencer_address: (),
                //         block_timestamp: (),
                //         protocol_version: (),
                //         l1_gas_price: (),
                //         l1_da_mode: (),
                //     },
                //     content: Default::default(),
                // })
                todo!()
            }
        }
    }

    pub fn get_preconfirmed_block_n(&self) -> u64 {
        self.preconfirmed.read().expect("Poisoned lock").block_n()
    }

    pub fn get_latest_block_n_(&self) -> Option<u64> {
        self.get_preconfirmed_block_n().checked_sub(1)
    }

    pub fn new_preconfirmed(self: &Arc<Self>, header: PendingHeader) -> Arc<PreconfirmedBlock> {
        let mut lock = self.preconfirmed.write().expect("Poisoned lock");
        // TODO: assert preconfirmed parent block hash
        let block_n = lock.block_n();
        let block = Arc::new(PreconfirmedBlock { block_n, header, content: Default::default() });
        *lock = PreconfirmedStatus::Preconfirmed(block.clone());
        block
    }
}
