use std::{
    cmp,
    collections::{hash_map, BTreeSet, HashMap, HashSet},
    iter,
    time::SystemTime,
};

use blockifier::transaction::account_transaction::AccountTransaction;
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
};

use crate::{contract_addr, nonce, tx_hash};

pub(crate) type ArrivedAtTimestamp = SystemTime;

pub(crate) struct MempoolTransaction {
    pub(crate) tx: AccountTransaction,
    pub(crate) arrived_at: ArrivedAtTimestamp,
}

impl MempoolTransaction {
    pub fn nonce(&self) -> Nonce {
        nonce(&self.tx)
    }
    pub fn contract_address(&self) -> ContractAddress {
        contract_addr(&self.tx)
    }
    pub fn tx_hash(&self) -> TransactionHash {
        tx_hash(&self.tx)
    }
}

struct OrderMempoolTransactionByNonce(MempoolTransaction);

impl PartialEq for OrderMempoolTransactionByNonce {
    fn eq(&self, other: &Self) -> bool {
        &self.0.nonce() == &other.0.nonce()
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
        Some(self.cmp(&other))
    }
}

/// Invariants:
/// - front_nonce, front_arrived_at and front_tx_hash must match the front transaction timestamp.
/// - No nonce chain should ever be empty in the mempool.
pub struct NonceChain {
    transactions: BTreeSet<OrderMempoolTransactionByNonce>,
    front_arrived_at: ArrivedAtTimestamp,
    front_tx_hash: TransactionHash,
}

#[derive(Eq, PartialEq)]
enum InsertedPosition {
    Front { former_head_arrived_at: ArrivedAtTimestamp },
    Other,
}

impl NonceChain {
    pub fn new_with_first_tx(tx: MempoolTransaction) -> Self {
        Self {
            front_arrived_at: tx.arrived_at,
            front_tx_hash: tx.tx_hash(),
            transactions: iter::once(OrderMempoolTransactionByNonce(tx)).collect(),
        }
    }

    pub fn check_invariants(&self) {
        debug_assert!(!self.transactions.is_empty());
        if cfg!(debug_assertions) {
            let front = self.transactions.first().unwrap();
            debug_assert_eq!(front.0.tx_hash(), self.front_tx_hash);
            debug_assert_eq!(front.0.arrived_at, self.front_arrived_at);
        }
    }

    /// Returns where in the chain it was inserted.
    pub fn insert(&mut self, mempool_tx: MempoolTransaction) -> Result<InsertedPosition, TxInsersionError> {
        let position = if self.front_arrived_at > mempool_tx.arrived_at {
            // We are inserting at the front here
            let former_head_arrived_at = self.front_arrived_at;
            self.front_arrived_at = mempool_tx.arrived_at;
            self.front_tx_hash = mempool_tx.tx_hash();
            InsertedPosition::Front { former_head_arrived_at }
        } else {
            InsertedPosition::Other
        };

        debug_assert_eq!(self.transactions.first().unwrap().0.tx_hash(), self.front_tx_hash);

        if !self.transactions.insert(OrderMempoolTransactionByNonce(mempool_tx)) {
            return Err(TxInsersionError::NonceConflict);
        }

        Ok(position)
    }
}

struct AccountOrderedByTimestamp {
    contract_addr: ContractAddress,
    timestamp: ArrivedAtTimestamp,
}

impl PartialEq for AccountOrderedByTimestamp {
    fn eq(&self, other: &Self) -> bool {
        // Important: Contract addr here, not timestamp
        self.contract_addr == other.contract_addr
    }
}
impl Eq for AccountOrderedByTimestamp {}
impl Ord for AccountOrderedByTimestamp {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}
impl PartialOrd for AccountOrderedByTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

#[derive(Default)]
/// Invariants:
/// - Every nonce chain in `nonce_chains` should have a one to one match with `tx_queue`.
/// - Every [`AccountTransaction::DeployAccount`] transaction should have a one to one match with `deployed_contracts`.
/// - See [`NonceChain`] invariants.
pub struct MempoolInner {
    /// We have one nonce chain per contract address.
    nonce_chains: HashMap<ContractAddress, NonceChain>,
    /// FCFS queue.
    tx_queue: BTreeSet<AccountOrderedByTimestamp>,
    /// This is used for quickly checking if the contract has been deployed for the same block it is invoked.
    deployed_contracts: HashSet<ContractAddress>,
}

#[derive(thiserror::Error, Debug)]
pub enum TxInsersionError {
    #[error("A transaction with this nonce already exists in the transaction pool")]
    NonceConflict,
    #[error("A transaction deploying the same account already exists in the transaction pool")]
    AccountAlreadyDeployed,
}

impl MempoolInner {
    pub fn insert_tx(&mut self, mempool_tx: MempoolTransaction) -> Result<(), TxInsersionError> {
        // Get the nonce chain for the contract

        let contract_addr = mempool_tx.contract_address();
        let arrived_at = mempool_tx.arrived_at;

        let deployed_contract_address =
            if let AccountTransaction::DeployAccount(tx) = &mempool_tx.tx { Some(tx.contract_address) } else { None };

        if let Some(contract_address) = &deployed_contract_address {
            if !self.deployed_contracts.insert(*contract_address) {
                return Err(TxInsersionError::AccountAlreadyDeployed);
            }
        }

        match self.nonce_chains.entry(contract_addr) {
            hash_map::Entry::Occupied(mut entry) => {
                // Handle nonce collision.
                let position = match entry.get_mut().insert(mempool_tx) {
                    Ok(position) => position,
                    Err(_nonce_collision) => {
                        // Rollback the prior mutation.
                        if let Some(contract_address) = &deployed_contract_address {
                            if !self.deployed_contracts.remove(contract_address) {
                                return Err(TxInsersionError::AccountAlreadyDeployed);
                            }
                        }

                        return Err(TxInsersionError::NonceConflict);
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
            }
            hash_map::Entry::Vacant(entry) => {
                // Insert the new nonce chain
                let nonce_chain = NonceChain::new_with_first_tx(mempool_tx);
                entry.insert(nonce_chain);

                // Also update the tx queue.
                let inserted = self.tx_queue.insert(AccountOrderedByTimestamp { contract_addr, timestamp: arrived_at });
                debug_assert!(inserted);
            }
        };
        Ok(())
    }

    pub fn has_deployed_contract(&self, addr: &ContractAddress) -> bool {
        self.deployed_contracts.contains(addr)
    }
}
