use crate::prelude::*;
use mp_block::{header::PreconfirmedHeader, Transaction, TransactionWithReceipt};
use mp_class::ConvertedClass;
use mp_state_update::TransactionStateUpdate;
use mp_transactions::TransactionWithHash;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct PreconfirmedExecutedTransaction {
    pub transaction: TransactionWithReceipt,
    pub state_diff: TransactionStateUpdate,
    pub declared_class: Option<ConvertedClass>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub enum PreconfirmedTransaction {
    Executed(PreconfirmedExecutedTransaction),
    Candidate(TransactionWithHash),
}

impl PreconfirmedTransaction {
    pub fn as_executed(&self) -> Option<&PreconfirmedExecutedTransaction> {
        match self {
            Self::Executed(tx) => Some(tx),
            _ => None,
        }
    }
    pub fn as_candidate(&self) -> Option<&TransactionWithHash> {
        match self {
            Self::Candidate(tx) => Some(tx),
            _ => None,
        }
    }
    pub fn transaction(&self) -> &Transaction {
        match self {
            Self::Executed(tx) => &tx.transaction.transaction,
            Self::Candidate(tx) => &tx.transaction,
        }
    }
    pub fn hash(&self) -> &Felt {
        match self {
            Self::Executed(tx) => tx.transaction.receipt.transaction_hash(),
            Self::Candidate(tx) => &tx.hash,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PreconfirmedBlockInner {
    txs: Vec<PreconfirmedTransaction>,
    n_executed: usize,
}

impl PreconfirmedBlockInner {
    pub fn all_transactions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &PreconfirmedTransaction> + Clone + ExactSizeIterator {
        self.txs[..].iter()
    }
    pub fn executed_transactions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &PreconfirmedExecutedTransaction> + Clone + ExactSizeIterator {
        self.txs[..self.n_executed].iter().map(|c: &PreconfirmedTransaction| {
            c.as_executed().expect("Invalid state: candidate transaction marked as executed")
        })
    }
    pub fn candidate_transactions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &TransactionWithHash> + Clone + ExactSizeIterator {
        self.txs[self.n_executed..].iter().map(|c: &PreconfirmedTransaction| {
            c.as_candidate().expect("Invalid state: executed transaction marked as candidate")
        })
    }

    /// Removes all candidate transactions.
    pub fn append_executed(&mut self, txs: impl IntoIterator<Item = PreconfirmedExecutedTransaction>) {
        self.txs.splice(self.n_executed.., txs.into_iter().map(PreconfirmedTransaction::Executed));
        self.n_executed = self.txs.len();
    }
    pub fn append_candidates(&mut self, txs: impl IntoIterator<Item = TransactionWithHash>) {
        self.txs.extend(txs.into_iter().map(PreconfirmedTransaction::Candidate))
    }

    pub fn n_executed(&self) -> usize {
        self.n_executed
    }
}

#[derive(Debug)]
pub struct PreconfirmedBlock {
    pub header: PreconfirmedHeader,
    /// We use a tokio watch channel here instead of a std RwLock, because we want to be able to
    /// listen for changes. Tokio watch acts basically as a wrapper around std RwLock, but with a tokio Notify
    /// alongside it.
    pub(crate) content: tokio::sync::watch::Sender<PreconfirmedBlockInner>,
}

impl PartialEq for PreconfirmedBlock {
    fn eq(&self, other: &Self) -> bool {
        // double borrow: it's a rwlock so there is no risk of reentrency deadlock if self and other are the object
        self.header == other.header && *self.content.borrow() == *other.content.borrow()
    }
}
impl Eq for PreconfirmedBlock {}

impl PreconfirmedBlock {
    pub fn new(header: PreconfirmedHeader) -> Self {
        Self { header, content: tokio::sync::watch::Sender::new(Default::default()) }
    }

    /// Replaces all candidate transactions with the content of `replace_candidates`.
    pub(crate) fn append(
        &self,
        executed: impl IntoIterator<Item = PreconfirmedExecutedTransaction>,
        replace_candidates: impl IntoIterator<Item = TransactionWithHash>,
    ) {
        // Takes the write lock and modify the content.
        self.content.send_modify(|block| {
            block.append_executed(executed);
            block.append_candidates(replace_candidates)
        });
    }
}
