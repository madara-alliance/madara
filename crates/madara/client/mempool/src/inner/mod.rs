//! The inner mempool does not perform validation, and is expected to be stored into a RwLock or Mutex.
//! This is the chokepoint for all insertions and popping, as such, we want to make it as fast as possible.
//! Insertion and popping should be O(log n).
//! We also really don't want to poison the lock by panicking.

use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use deployed_contracts::DeployedContracts;
use mp_convert::ToFelt;
use nonce_chain::{InsertedPosition, NonceChain, NonceChainNewState, ReplacedState};
use starknet_api::core::ContractAddress;
use starknet_types_core::felt::Felt;
use std::{
    cmp,
    collections::{hash_map, BTreeSet, HashMap},
};

mod deployed_contracts;
mod limits;
mod nonce_chain;
mod proptest;
mod tx;

pub use limits::*;
pub use tx::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AccountOrderedByTimestamp {
    contract_addr: Felt,
    timestamp: ArrivedAtTimestamp,
}

impl Ord for AccountOrderedByTimestamp {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // Important: Fallback on contract addr here.
        // There can be timestamp collisions.
        self.timestamp.cmp(&other.timestamp).then_with(|| self.contract_addr.cmp(&other.contract_addr))
    }
}
impl PartialOrd for AccountOrderedByTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
/// Invariants:
/// - Every nonce chain in `nonce_chains` should have a one to one match with `tx_queue`.
/// - Every [`AccountTransaction::DeployAccount`] transaction should have a one to one match with `deployed_contracts`.
/// - See [`NonceChain`] invariants.
pub(crate) struct MempoolInner {
    /// We have one nonce chain per contract address.
    nonce_chains: HashMap<Felt, NonceChain>,
    /// FCFS queue.
    tx_queue: BTreeSet<AccountOrderedByTimestamp>,
    deployed_contracts: DeployedContracts,
    limiter: MempoolLimiter,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum TxInsersionError {
    #[error("A transaction with this nonce already exists in the transaction pool")]
    NonceConflict,
    #[error("A transaction with this hash already exists in the transaction pool")]
    DuplicateTxn,
    #[error(transparent)]
    Limit(#[from] MempoolLimitReached),
}

impl MempoolInner {
    pub fn new(limits_config: MempoolLimits) -> Self {
        Self {
            nonce_chains: Default::default(),
            tx_queue: Default::default(),
            deployed_contracts: Default::default(),
            limiter: MempoolLimiter::new(limits_config),
        }
    }

    #[cfg(test)]
    pub fn check_invariants(&self) {
        self.nonce_chains.values().for_each(NonceChain::check_invariants);
        let mut tx_queue = self.tx_queue.clone();
        for (k, v) in &self.nonce_chains {
            assert!(tx_queue.remove(&AccountOrderedByTimestamp { contract_addr: *k, timestamp: v.front_arrived_at }))
        }
        assert_eq!(tx_queue, Default::default());
        let mut deployed_contracts = self.deployed_contracts.clone();
        for (contract, _) in self.nonce_chains.values().flat_map(|chain| &chain.transactions) {
            if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &contract.0.tx {
                deployed_contracts.decrement(tx.contract_address)
            }
        }
        assert!(deployed_contracts.is_empty(), "remaining deployed_contracts: {deployed_contracts:?}");
    }

    /// When `force` is `true`, this function should never return any error.
    /// `update_limits` is `false` when the transaction has been removed from the mempool in the past without updating the limits.
    pub fn insert_tx(
        &mut self,
        mempool_tx: MempoolTransaction,
        force: bool,
        update_limits: bool,
    ) -> Result<(), TxInsersionError> {
        // delete age-exceeded txs from the mempool
        // todo(perf): this may want to limit this check once every few seconds to avoid it being in the hot path?
        self.remove_age_exceeded_txs();

        // check limits
        let limits_for_tx = TransactionCheckedLimits::limits_for(&mempool_tx);
        if !force {
            self.limiter.check_insert_limits(&limits_for_tx)?;
        }

        let contract_addr = mempool_tx.contract_address().to_felt();
        let arrived_at = mempool_tx.arrived_at;
        let deployed_contract_address =
            if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &mempool_tx.tx {
                Some(tx.contract_address)
            } else {
                None
            };

        let is_replaced = match self.nonce_chains.entry(contract_addr) {
            hash_map::Entry::Occupied(mut entry) => {
                // Handle nonce collision.
                let chain: &mut NonceChain = entry.get_mut();
                let (position, is_replaced) = match chain.insert(mempool_tx, force) {
                    Ok(position) => position,
                    Err(nonce_collision_or_duplicate_hash) => {
                        debug_assert!(!force); // "Force add should never error
                        return Err(nonce_collision_or_duplicate_hash);
                    }
                };

                match position {
                    InsertedPosition::Front { former_head_arrived_at } => {
                        // If we inserted at the front, it has invalidated the tx queue. Update the tx queue.
                        let removed = self
                            .tx_queue
                            .remove(&AccountOrderedByTimestamp { contract_addr, timestamp: former_head_arrived_at });
                        debug_assert!(removed);
                        let inserted =
                            self.tx_queue.insert(AccountOrderedByTimestamp { contract_addr, timestamp: arrived_at });
                        debug_assert!(inserted);
                    }
                    InsertedPosition::Other => {
                        // No need to update the tx queue.
                    }
                }
                is_replaced
            }
            hash_map::Entry::Vacant(entry) => {
                // Insert the new nonce chain
                let nonce_chain = NonceChain::new_with_first_tx(mempool_tx);
                entry.insert(nonce_chain);

                // Also update the tx queue.
                let inserted = self.tx_queue.insert(AccountOrderedByTimestamp { contract_addr, timestamp: arrived_at });
                debug_assert!(inserted);

                ReplacedState::NotReplaced
            }
        };

        if let ReplacedState::Replaced { previous } = is_replaced {
            // Mark the previous transaction as deleted
            self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&previous));
        } else if let Some(contract_address) = &deployed_contract_address {
            self.deployed_contracts.increment(*contract_address)
        }

        // Update transaction limits
        if update_limits {
            self.limiter.update_tx_limits(&limits_for_tx);
        }

        Ok(())
    }

    pub fn has_deployed_contract(&self, addr: &ContractAddress) -> bool {
        self.deployed_contracts.contains(addr)
    }

    fn pop_tx_queue_account(&mut self, tx_queue_account: &AccountOrderedByTimestamp) -> MempoolTransaction {
        // Update nonce chain.
        let nonce_chain =
            self.nonce_chains.get_mut(&tx_queue_account.contract_addr).expect("Nonce chain does not match tx queue");
        let (mempool_tx, nonce_chain_new_state) = nonce_chain.pop();
        match nonce_chain_new_state {
            NonceChainNewState::Empty => {
                // Remove the nonce chain.
                let removed = self.nonce_chains.remove(&tx_queue_account.contract_addr);
                debug_assert!(removed.is_some());
            }
            NonceChainNewState::NotEmpty => {
                // Re-add to tx queue.
                let inserted = self.tx_queue.insert(AccountOrderedByTimestamp {
                    contract_addr: tx_queue_account.contract_addr,
                    timestamp: nonce_chain.front_arrived_at,
                });
                debug_assert!(inserted);
            }
        }

        // Update deployed contracts.
        if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &mempool_tx.tx {
            self.deployed_contracts.decrement(tx.contract_address);
        }

        mempool_tx
    }

    pub fn remove_age_exceeded_txs(&mut self) {
        // Pop tx queue.
        // too bad there's no first_entry api, we should check if hashbrown has it to avoid the double lookup.
        while let Some(tx_queue_account) = self.tx_queue.first() {
            let tx_queue_account = tx_queue_account.clone(); // clone is cheap for this struct
            let nonce_chain = self
                .nonce_chains
                .get_mut(&tx_queue_account.contract_addr)
                .expect("Nonce chain does not match tx queue");
            let (k, _v) = nonce_chain.transactions.first_key_value().expect("Nonce chain without a tx");

            if self.limiter.tx_age_exceeded(&TransactionCheckedLimits::limits_for(&k.0)) {
                let tx = self.pop_tx_queue_account(&tx_queue_account);
                let _res = self.tx_queue.pop_first().expect("Cannot be empty, checked just above");
                self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&tx));
            } else {
                break;
            }
        }
    }

    pub fn pop_next(&mut self) -> Option<MempoolTransaction> {
        // Pop tx queue.
        let mempool_tx = loop {
            let tx_queue_account = self.tx_queue.pop_first()?; // Bubble up None if the mempool is empty.
            let mempool_tx = self.pop_tx_queue_account(&tx_queue_account);

            let limits = TransactionCheckedLimits::limits_for(&mempool_tx);
            if !self.limiter.tx_age_exceeded(&limits) {
                break mempool_tx;
            }

            // transaction age exceeded, remove the tx from mempool.
            self.limiter.mark_removed(&limits);
        };

        // do not update mempool limits, block prod will update it with re-add txs.
        Some(mempool_tx)
    }

    pub fn pop_next_chunk(&mut self, dest: &mut impl Extend<MempoolTransaction>, n: usize) {
        dest.extend((0..n).map_while(|_| self.pop_next()))
    }

    /// This is called by the block production after a batch of transaction is executed.
    /// Mark the consumed txs as consumed, and re-add the transactions that are not consumed in the mempool.
    pub fn re_add_txs(
        &mut self,
        txs: impl IntoIterator<Item = MempoolTransaction>,
        consumed_txs: impl IntoIterator<Item = MempoolTransaction>,
    ) {
        for tx in consumed_txs {
            self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&tx))
        }
        for tx in txs {
            let force = true;
            self.insert_tx(tx, force, false).expect("Force insert tx should not error");
        }
    }

    /// This is called by the block production after a batch of transaction is executed.
    /// Mark the consumed txs as consumed, and re-add the transactions that are not consumed in the mempool.
    pub fn insert_txs(
        &mut self,
        txs: impl IntoIterator<Item = MempoolTransaction>,
        force: bool,
    ) -> Result<(), TxInsersionError> {
        for tx in txs {
            self.insert_tx(tx, force, true)?;
        }
        Ok(())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_empty(&self) -> bool {
        self.tx_queue.is_empty()
    }
}
