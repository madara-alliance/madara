use super::tx::MempoolTransaction;
use crate::TxInsertionError;
use starknet_api::core::Nonce;
use std::collections::{btree_map, BTreeMap};
use std::iter;

/// A wrapper around a [BTreeMap] which provides a mapping from a [Nonce] to the
/// associated transaction.
#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(Clone))]
pub struct NonceTxMapping {
    /// An ordered mapping of the transactions to come from an account, accessed
    /// by [Nonce].
    pub(crate) transactions: BTreeMap<Nonce, MempoolTransaction>,
}

#[derive(Debug)]
pub enum ReplacedState {
    Replaced { previous: MempoolTransaction },
    NotReplaced,
}

#[derive(Eq, PartialEq, Debug)]
pub enum NonceTxMappingNewState {
    Empty,
    NotEmpty,
}

impl NonceTxMapping {
    pub fn new_with_first_tx(tx: MempoolTransaction, nonce: Nonce) -> Self {
        Self { transactions: iter::once((nonce, tx)).collect() }
    }

    /// Returns where in the chain it was inserted.
    /// When `force` is `true`, this function should never return any error.
    pub fn insert(
        &mut self,
        mempool_tx: MempoolTransaction,
        nonce: Nonce,
        force: bool,
    ) -> Result<ReplacedState, TxInsertionError> {
        let replaced = if force {
            match self.transactions.entry(nonce) {
                btree_map::Entry::Vacant(entry) => {
                    entry.insert(mempool_tx);
                    ReplacedState::NotReplaced
                }
                btree_map::Entry::Occupied(mut entry) => {
                    let previous = entry.insert(mempool_tx);
                    ReplacedState::Replaced { previous }
                }
            }
        } else {
            match self.transactions.entry(nonce) {
                btree_map::Entry::Occupied(entry) => {
                    // duplicate nonce, either it's because the hash is
                    // duplicated or nonce conflict with another tx.
                    if entry.get().tx_hash() == mempool_tx.tx_hash() {
                        return Err(TxInsertionError::DuplicateTxn);
                    } else {
                        return Err(TxInsertionError::NonceConflict);
                    }
                }
                btree_map::Entry::Vacant(entry) => {
                    entry.insert(mempool_tx);
                    ReplacedState::NotReplaced
                }
            }
        };

        Ok(replaced)
    }

    pub fn pop(&mut self) -> (MempoolTransaction, NonceTxMappingNewState) {
        let (_, tx) = self.transactions.pop_first().expect("Nonce chain should not be empty");
        if self.transactions.is_empty() {
            (tx, NonceTxMappingNewState::Empty)
        } else {
            (tx, NonceTxMappingNewState::NotEmpty)
        }
    }
}
