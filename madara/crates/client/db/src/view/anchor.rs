use crate::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedBlockInner, PreconfirmedExecutedTransaction},
    prelude::*,
    storage::MadaraStorageRead,
    MadaraBackend,
};
use mp_transactions::TransactionWithHash;
use std::fmt;

/// Lock guard on the content of a preconfirmed block. Only the first [`n_txs_visible`] executed transactions
/// are visible.
pub(crate) struct PreconfirmedBlockAnchorRef<'a> {
    view: tokio::sync::watch::Ref<'a, PreconfirmedBlockInner>,
    /// Only executed transactions are visible in the anchor.
    n_txs_visible: usize,
}

impl<'a> PreconfirmedBlockAnchorRef<'a> {
    pub fn executed_transactions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &PreconfirmedExecutedTransaction> + Clone + ExactSizeIterator {
        self.view.executed_transactions().take(self.n_txs_visible)
    }
}

/// Note: The Eq/PartialEq implementation uses Arc::ptr_eq only. Two preconfirmed blocks with the same content
/// and header will appear as different preconfirmed blocks if they are not alias of one another.
#[derive(Debug, Clone)]
pub struct PreconfirmedBlockAnchor {
    /// Number of transactions visible in the block.
    n_txs_visible: usize,
    pub(crate) block: Arc<PreconfirmedBlock>,
    pub(crate) block_content: tokio::sync::watch::Receiver<PreconfirmedBlockInner>,

    /// Candidate transactions. Most of the time, we don't care about those, so this vec is empty.
    /// This vec is only filled when using `reset_with_candidates`.
    pub(crate) candidates: Vec<TransactionWithHash>,
}

impl PartialEq for PreconfirmedBlockAnchor {
    fn eq(&self, other: &Self) -> bool {
        self.n_txs_visible == other.n_txs_visible && Arc::ptr_eq(&self.block, &other.block)
    }
}
impl Eq for PreconfirmedBlockAnchor {}

impl PreconfirmedBlockAnchor {
    /// New view on block start (no transaction).
    pub(crate) fn new_at_start(block: Arc<PreconfirmedBlock>) -> Self {
        let mut block_content = block.content.subscribe();
        block_content.mark_changed(); // mark outdated
        Self { n_txs_visible: 0, block_content, block, candidates: vec![] }
    }
    /// New view on the current block state.
    pub(crate) fn new(block: Arc<PreconfirmedBlock>) -> Self {
        let mut this = Self::new_at_start(block);
        this.refresh();
        this
    }
    /// Returns a lock guard into the current block content.
    pub(crate) fn borrow_content(&self) -> PreconfirmedBlockAnchorRef<'_> {
        PreconfirmedBlockAnchorRef { view: self.block_content.borrow(), n_txs_visible: self.n_txs_visible }
    }

    /// Make the new executed transactions visible.
    pub(crate) fn refresh(&mut self) {
        self.n_txs_visible = self.block_content.borrow_and_update().n_executed();
        self.candidates.clear();
    }

    /// Make the new transactions visible. Candidate transactions will also be visible.
    pub(crate) fn refresh_with_candidates(&mut self) {
        let borrow = self.block_content.borrow_and_update();
        self.n_txs_visible = borrow.n_executed();
        self.candidates.clear();
        self.candidates.extend(borrow.candidate_transactions().cloned());
    }

    /// Returns when the block content has changed. Returns immediately if the view is
    /// already outdated. The view is not updated; you need to call [`Self::refresh`] or
    /// [`Self::refresh_with_candidates`] when this function returns.
    pub(crate) async fn wait_until_outdated(&mut self) {
        self.block_content.changed().await.expect("Channel unexpectedly closed");
        self.block_content.mark_changed();
    }

    pub(crate) async fn wait_next_tx(&mut self) -> PreconfirmedBlockChange {
        loop {
            {
                let borrow = self.block_content.borrow();
                // New preconfirmed transactions
                if borrow.n_executed() > self.n_txs_visible {
                    let transaction_index = self.n_txs_visible as u64;
                    self.n_txs_visible += 1;
                    return PreconfirmedBlockChange::NewPreconfirmed {
                        transaction_index,
                        // Clear all candidates.
                        removed_candidates: mem::take(&mut self.candidates),
                    };
                }
                // New candidate transaction
                if let Some(candidate) = borrow.candidate_transactions().nth(self.candidates.len()) {
                    let transaction_index = (self.n_txs_visible + self.candidates.len()) as u64;
                    self.candidates.push(candidate.clone());
                    return PreconfirmedBlockChange::NewCandidate { transaction_index };
                };
            }
            self.block_content.changed().await.expect("Channel unexpectedly closed");
        }
    }
    pub fn n_executed(&self) -> usize {
        self.n_txs_visible
    }
}

#[derive(Debug)]
pub enum PreconfirmedBlockChange {
    NewPreconfirmed { transaction_index: u64, removed_candidates: Vec<TransactionWithHash> },
    NewCandidate { transaction_index: u64 },
}

impl PreconfirmedBlockChange {
    pub fn transaction_index(&self) -> u64 {
        match self {
            Self::NewPreconfirmed { transaction_index, .. } => *transaction_index,
            Self::NewCandidate { transaction_index } => *transaction_index,
        }
    }
    pub fn removed_candidates(&mut self) -> Vec<TransactionWithHash> {
        match self {
            Self::NewPreconfirmed { removed_candidates, .. } => mem::take(removed_candidates),
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockAnchor {
    Preconfirmed(PreconfirmedBlockAnchor),
    Confirmed(u64),
}

impl BlockAnchor {
    pub fn new_on_preconfirmed(block: Arc<PreconfirmedBlock>) -> Self {
        Self::Preconfirmed(PreconfirmedBlockAnchor::new(block))
    }
    pub fn new_at_preconfirmed_start(block: Arc<PreconfirmedBlock>) -> Self {
        Self::Preconfirmed(PreconfirmedBlockAnchor::new_at_start(block))
    }
    pub fn new_on_confirmed(confirmed_block_n: u64) -> Self {
        Self::Confirmed(confirmed_block_n)
    }

    pub fn block_n(&self) -> u64 {
        match self {
            Self::Preconfirmed(preconfirmed_block_view) => preconfirmed_block_view.block.header.block_number,
            Self::Confirmed(block_n) => *block_n,
        }
    }
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        match self {
            Self::Preconfirmed(b) => b.block.header.block_number.checked_sub(1),
            Self::Confirmed(block_n) => Some(*block_n),
        }
    }
    pub fn is_preconfirmed(&self) -> bool {
        matches!(self, Self::Preconfirmed(_))
    }

    pub fn as_preconfirmed(&self) -> Option<&PreconfirmedBlockAnchor> {
        match self {
            Self::Preconfirmed(b) => Some(b),
            _ => None,
        }
    }
    pub fn into_preconfirmed(&self) -> Option<PreconfirmedBlockAnchor> {
        match self {
            Self::Preconfirmed(b) => Some(b.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Anchor {
    #[default]
    Empty,
    Block(BlockAnchor),
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "[Empty pre-genesis state]"),
            Self::Block(block_anchor) => write!(f, "{block_anchor}"),
        }
    }
}
impl fmt::Display for BlockAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preconfirmed(preconfirmed_block_anchor) => write!(f, "{preconfirmed_block_anchor}"),
            Self::Confirmed(confirmed) => write!(f, "[Confirmed block at height #{confirmed}]"),
        }
    }
}
impl fmt::Display for PreconfirmedBlockAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.candidates.is_empty() {
            write!(
                f,
                "[Preconfirmed block at height #{} with {} transactions]",
                self.block.header.block_number,
                self.n_executed()
            )
        } else {
            write!(
                f,
                "[Preconfirmed block at height #{} with {} transactions and {} candidates]",
                self.block.header.block_number,
                self.n_executed(),
                self.candidates.len()
            )
        }
    }
}

impl Anchor {
    pub fn new_on_preconfirmed(block: Arc<PreconfirmedBlock>) -> Self {
        Self::Block(BlockAnchor::new_on_preconfirmed(block))
    }
    pub fn new_on_confirmed(confirmed_block_n: Option<u64>) -> Self {
        if let Some(block_n) = confirmed_block_n {
            Self::Block(BlockAnchor::Confirmed(block_n))
        } else {
            Self::Empty
        }
    }

    pub(crate) fn preconfirmed(&self) -> Option<&PreconfirmedBlockAnchor> {
        match self {
            Self::Block(b) => b.as_preconfirmed(),
            _ => None,
        }
    }

    pub fn is_preconfirmed(&self) -> bool {
        match self {
            Self::Block(inner) => inner.is_preconfirmed(),
            _ => false,
        }
    }
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        match self {
            Self::Block(inner) => inner.latest_confirmed_block_n(),
            _ => None,
        }
    }
    pub fn block_n(&self) -> Option<u64> {
        match self {
            Self::Block(inner) => Some(inner.block_n()),
            _ => None,
        }
    }

    pub fn into_block_anchor(self) -> Option<BlockAnchor> {
        match self {
            Self::Block(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_block_anchor(&self) -> Option<&BlockAnchor> {
        match self {
            Self::Block(a) => Some(a),
            _ => None,
        }
    }

    /// Get the latest confirmed block visible from this anchor. If the anchor is on a confirmed block,
    /// it returns the same anchor. If the anchor is on a preconfirmed block, this returns the anchor pointing
    /// to its parent block.
    pub fn latest_confirmed(&self) -> Anchor {
        if let Some(preconfirmed) = self.preconfirmed() {
            Anchor::new_on_confirmed(preconfirmed.block.header.block_number.checked_sub(1))
        } else {
            self.clone()
        }
    }
}

pub trait IntoAnchor: Sized {
    fn into_anchor<DB: MadaraStorageRead>(self, backend: &MadaraBackend<DB>) -> Result<Option<Anchor>>;
    fn into_block_anchor<DB: MadaraStorageRead>(self, backend: &MadaraBackend<DB>) -> Result<Option<BlockAnchor>> {
        Ok(self.into_anchor(backend)?.and_then(|anchor| anchor.into_block_anchor()))
    }
}
impl IntoAnchor for Anchor {
    fn into_anchor<DB: MadaraStorageRead>(self, _backend: &MadaraBackend<DB>) -> Result<Option<Anchor>> {
        Ok(Some(self))
    }
}
impl IntoAnchor for BlockAnchor {
    fn into_anchor<DB: MadaraStorageRead>(self, _backend: &MadaraBackend<DB>) -> Result<Option<Anchor>> {
        Ok(Some(Anchor::Block(self)))
    }
}

impl IntoAnchor for mp_block::BlockId {
    fn into_anchor<DB: MadaraStorageRead>(self, backend: &MadaraBackend<DB>) -> Result<Option<Anchor>> {
        match self {
            mp_rpc::BlockId::Tag(mp_rpc::BlockTag::Pending) => {
                Ok(Some(Anchor::new_on_preconfirmed(backend.get_preconfirmed_or_fake())))
            }
            mp_rpc::BlockId::Tag(mp_rpc::BlockTag::Latest) => {
                Ok(Some(Anchor::new_on_confirmed(backend.latest_confirmed_block_n())))
            }
            mp_rpc::BlockId::Hash(hash) => {
                if let Some(on_block_n) = backend.db.find_block_hash(&hash)? {
                    Ok(Some(Anchor::new_on_confirmed(Some(on_block_n))))
                } else {
                    Ok(None)
                }
            }
            mp_rpc::BlockId::Number(block_n) => {
                if backend.latest_confirmed_block_n().is_some_and(|latest| latest >= block_n) {
                    Ok(Some(Anchor::new_on_confirmed(Some(block_n))))
                } else {
                    Ok(None)
                }
            }
        }
    }
}
