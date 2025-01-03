use super::tx::{ArrivedAtTimestamp, MempoolTransaction};
use crate::TxInsersionError;
use starknet_api::{core::Nonce, transaction::TransactionHash};
use std::collections::{btree_map, BTreeMap};
use std::{cmp, iter};

#[derive(Debug, Clone)]
pub struct OrderMempoolTransactionByNonce(pub MempoolTransaction);

impl PartialEq for OrderMempoolTransactionByNonce {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for OrderMempoolTransactionByNonce {}
impl Ord for OrderMempoolTransactionByNonce {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.nonce().cmp(&other.0.nonce())
    }
}
impl PartialOrd for OrderMempoolTransactionByNonce {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Invariants:
/// - front_nonce, front_arrived_at and front_tx_hash must match the front transaction timestamp.
/// - No nonce chain should ever be empty in the mempool.
#[derive(Debug)]
pub struct NonceChain {
    /// Use a BTreeMap to so that we can use the entry api.
    // TODO(perf): to avoid some double lookups here, we should remove the `OrderMempoolTransactionByNonce` struct
    // and make this a BTreeMap<Nonce, MempoolTransaction>
    pub(crate) transactions: BTreeMap<OrderMempoolTransactionByNonce, ()>,
    pub(crate) front_arrived_at: ArrivedAtTimestamp,
    pub(crate) front_nonce: Nonce,
    pub(crate) front_tx_hash: TransactionHash,
}

#[derive(Eq, PartialEq, Debug)]
pub enum InsertedPosition {
    Front { former_head_arrived_at: ArrivedAtTimestamp },
    Other,
}

#[derive(Debug)]
pub enum ReplacedState {
    Replaced { previous: MempoolTransaction },
    NotReplaced,
}

#[derive(Eq, PartialEq, Debug)]
pub enum NonceChainNewState {
    Empty,
    NotEmpty,
}

impl NonceChain {
    pub fn new_with_first_tx(tx: MempoolTransaction) -> Self {
        Self {
            front_arrived_at: tx.arrived_at,
            front_tx_hash: tx.tx_hash(),
            front_nonce: tx.nonce(),
            transactions: iter::once((OrderMempoolTransactionByNonce(tx), ())).collect(),
        }
    }

    #[cfg(test)]
    pub fn check_invariants(&self) {
        assert!(!self.transactions.is_empty());
        let (front, _) = self.transactions.first_key_value().unwrap();
        assert_eq!(front.0.tx_hash(), self.front_tx_hash);
        assert_eq!(front.0.nonce(), self.front_nonce);
        assert_eq!(front.0.arrived_at, self.front_arrived_at);
    }

    /// Returns where in the chain it was inserted.
    /// When `force` is `true`, this function should never return any error.
    pub fn insert(
        &mut self,
        mempool_tx: MempoolTransaction,
        force: bool,
    ) -> Result<(InsertedPosition, ReplacedState), TxInsersionError> {
        let mempool_tx_arrived_at = mempool_tx.arrived_at;
        let mempool_tx_nonce = mempool_tx.nonce();
        let mempool_tx_hash = mempool_tx.tx_hash();

        let replaced = if force {
            // double lookup here unfortunately.. that's because we're using the keys in a hacky way and can't update the
            // entry key using the entry api.
            let mempool_tx = OrderMempoolTransactionByNonce(mempool_tx);
            if let Some((previous, _)) = self.transactions.remove_entry(&mempool_tx) {
                let previous = previous.0.clone();
                let inserted = self.transactions.insert(mempool_tx, ());
                debug_assert!(inserted.is_none());
                ReplacedState::Replaced { previous }
            } else {
                let inserted = self.transactions.insert(mempool_tx, ());
                debug_assert!(inserted.is_none());
                ReplacedState::NotReplaced
            }
        } else {
            match self.transactions.entry(OrderMempoolTransactionByNonce(mempool_tx)) {
                btree_map::Entry::Occupied(entry) => {
                    // duplicate nonce, either it's because the hash is duplicated or nonce conflict with another tx.
                    if entry.key().0.tx_hash() == mempool_tx_hash {
                        return Err(TxInsersionError::DuplicateTxn);
                    } else {
                        return Err(TxInsersionError::NonceConflict);
                    }
                }
                btree_map::Entry::Vacant(entry) => *entry.insert(()),
            }

            ReplacedState::NotReplaced
        };

        let position = if self.front_nonce >= mempool_tx_nonce {
            // We insrted at the front here
            let former_head_arrived_at = core::mem::replace(&mut self.front_arrived_at, mempool_tx_arrived_at);
            self.front_nonce = mempool_tx_nonce;
            self.front_tx_hash = mempool_tx_hash;
            InsertedPosition::Front { former_head_arrived_at }
        } else {
            InsertedPosition::Other
        };

        debug_assert_eq!(
            self.transactions.first_key_value().expect("Getting the first tx").0 .0.tx_hash(),
            self.front_tx_hash
        );

        Ok((position, replaced))
    }

    pub fn pop(&mut self) -> (MempoolTransaction, NonceChainNewState) {
        // TODO(perf): avoid double lookup
        let (tx, _) = self.transactions.pop_first().expect("Nonce chain should not be empty");
        if let Some((new_front, _)) = self.transactions.first_key_value() {
            self.front_arrived_at = new_front.0.arrived_at;
            self.front_tx_hash = new_front.0.tx_hash();
            self.front_nonce = new_front.0.nonce();
            (tx.0, NonceChainNewState::NotEmpty)
        } else {
            (tx.0, NonceChainNewState::Empty)
        }
    }
}
