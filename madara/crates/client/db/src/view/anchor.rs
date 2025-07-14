use mp_block::EventWithInfo;

use crate::{
    db::DBBackend,
    view::{PreconfirmedBlock, PreconfirmedBlockInnerView, PreconfirmedBlockTransaction},
    MadaraBackend, MadaraStorageError,
};
use std::{ops::Deref, sync::Arc};

pub(crate) struct PreconfirmedBlockViewContent<'a> {
    block: &'a PreconfirmedBlock,
    view: PreconfirmedBlockInnerView<'a>,
    n_txs: u64,
}

impl<'a> PreconfirmedBlockViewContent<'a> {
    pub fn transactions(&self) -> &[PreconfirmedBlockTransaction] {
        &self.view.transactions()[..self.n_txs as usize]
    }

    pub fn events_with_info(&self) -> impl Iterator<Item = EventWithInfo> {
        block
            .inner
            .receipts
            .into_iter()
            .enumerate()
            .flat_map(|(transaction_index, receipt)| {
                let transaction_hash = receipt.transaction_hash();
                receipt.events().iter().map(move |events| (transaction_index, transaction_hash, events))
            })
            .enumerate()
            .map(move |(event_index, (transaction_index, transaction_hash, event))| EventWithInfo {
                event,
                block_number: self.block.block_n,
                event_index_in_block: event_index as _,
                block_hash,
                transaction_hash: None,
                transaction_index: transaction_index as _,
                in_preconfirmed: true
            })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreconfirmedBlockView {
    pub block: Arc<PreconfirmedBlock>,
    pub n_txs: u64,
}

impl PreconfirmedBlockView {
    pub fn content(&self) -> PreconfirmedBlockViewContent<'_> {
        PreconfirmedBlockViewContent { view: self.block.content(), n_txs: self.n_txs, block: &self.block }
    }
}

#[derive(Debug, Clone)]
pub enum BlockAnchor {
    Preconfirmed(PreconfirmedBlockView),
    OnBlockN(u64),
}

impl BlockAnchor {
    pub fn new_on_preconfirmed(block: Arc<PreconfirmedBlock>) -> Self {
        let on_block_n = block.parent_block_n();
        let n_txs = block.n_txs();
        Self::Preconfirmed(PreconfirmedBlockView { block, n_txs })
    }
    pub fn new_on_block_n(block_n: u64) -> Self {
        Self::OnBlockN(block_n)
    }

    pub fn block_n(&self) -> u64 {
        match self {
            Self::Preconfirmed(preconfirmed_block_view) => preconfirmed_block_view.block.block_n,
            Self::OnBlockN(block_n) => block_n,
        }
    }

    pub(crate) fn preconfirmed(&self) -> Option<&PreconfirmedBlockView> {
        match self {
            Self::Preconfirmed(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Anchor {
    Block(BlockAnchor),
    Empty,
}

impl Anchor {
    pub fn new_on_preconfirmed(block: Arc<PreconfirmedBlock>) -> Self {
        Self::Block(BlockAnchor::new_on_preconfirmed(block))
    }
    pub fn new_on_block_n(on_block_n: Option<u64>) -> Self {
        if let Some(block_n) = on_block_n {
            Self::Block(BlockAnchor::OnBlockN(block_n))
        } else {
            Self::Empty
        }
    }

    pub(crate) fn preconfirmed(&self) -> Option<&PreconfirmedBlockView> {
        match self {
            Self::Block(b) => b.preconfirmed(),
            _ => None,
        }
    }
    pub(crate) fn on_block_n(&self) -> Option<u64> {
        match self {
            Self::Block(Self::Preconfirmed(b)) => b.block.block_n.checked_sub(1),
            Self::Block(Self::OnBlockN(block_n)) => Some(block_n),
            _ => None,
        }
    }

    pub fn into_block_anchor(self) -> Option<BlockAnchor> {
        match self {
            Self::Block(a) => Some(a),
            _ => None,
        }
    }
}

pub trait IntoAnchor {
    fn into_anchor<DB: DBBackend>(self, backend: &MadaraBackend<DB>) -> Result<Option<Anchor>, MadaraStorageError>;
    fn into_block_anchor<DB: DBBackend>(
        self,
        backend: &MadaraBackend<DB>,
    ) -> Result<Option<BlockAnchor>, MadaraStorageError> {
        self.into_anchor(backend)?.and_then(|anchor| anchor.into_block_anchor())
    }
}

impl IntoAnchor for mp_block::BlockId {
    fn into_anchor<DB: DBBackend>(self, backend: &MadaraBackend<DB>) -> Result<Option<Anchor>, MadaraStorageError> {
        match self {
            mp_rpc::BlockId::Tag(mp_rpc::BlockTag::Pending) => {
                let block = backend.get_preconfirmed().clone();
                Ok(Some(Anchor::new_on_preconfirmed(block)))
            }
            mp_rpc::BlockId::Tag(mp_rpc::BlockTag::Latest) => {
                Ok(Some(Anchor::new_on_block_n(backend.get_latest_block_n_())))
            }
            mp_rpc::BlockId::Hash(hash) => {
                if let Some(on_block_n) = backend.db.find_block_hash(&hash)? {
                    Ok(Some(Anchor::new_on_block_n(Some(on_block_n))))
                } else {
                    Ok(None)
                }
            }
            mp_rpc::BlockId::Number(block_n) => {
                if backend.get_latest_block_n_().is_some_and(|latest| latest >= block_n) {
                    Ok(Some(Anchor::new_on_block_n(Some(block_n))))
                } else {
                    Ok(None)
                }
            }
        }
    }
}
