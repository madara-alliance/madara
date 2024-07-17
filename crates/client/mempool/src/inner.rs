use std::{
    cmp,
    collections::{hash_map, BTreeSet, BinaryHeap, HashMap, HashSet},
    time::SystemTime,
};

use blockifier::transaction::account_transaction::AccountTransaction;
use starknet_api::core::{ContractAddress, Nonce};

use crate::{contract_addr, nonce};

type ArrivedAtTimestamp = SystemTime;

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

/// Invariant: arrived_at must match the next transaction timestamp.
pub struct NonceChain {
    transactions: BTreeSet<OrderMempoolTransactionByNonce>,
    arrived_at: ArrivedAtTimestamp,
}

enum InsertedPosition {
    Front,
    Other,
}

impl NonceChain {
    /// # Panics
    ///
    /// Panics if the NonceChain is empty.
    pub fn pop_head(&mut self) -> MempoolTransaction {
        self.transactions.pop_first().map(|tx| tx.0).expect("No transaction for this queue item")
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    /// # Panics
    ///
    /// Panics if the NonceChain is empty.
    pub fn next_timestamp(&self) -> ArrivedAtTimestamp {
        self.arrived_at
    }

    pub fn insert(&mut self, mempool_tx: MempoolTransaction) -> Result<InsertedPosition, TxInsersionError> {
        if !self.transactions.insert(OrderMempoolTransactionByNonce(mempool_tx)) {
            Err(TxInsersionError::NonceConflict)
        }
        self.transactions
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
/// TODO: document invariants
pub struct MempoolInner {
    /// We have one nonce chain per contract address.
    nonce_chains: HashMap<ContractAddress, NonceChain>,
    /// FCFS queue.
    tx_queue: BinaryHeap<AccountOrderedByTimestamp>,
    /// This is used for quickly checking if the contract has been deployed for the same block it is invoked.
    deployed_contracts: HashSet<ContractAddress>,
}

#[derive(thiserror::Error, Debug)]
pub enum TxInsersionError {
    #[error("A transaction with this nonce already exists in the transaction pool")]
    NonceConflict,
}

impl MempoolInner {
    pub fn insert_tx(&mut self, tx: MempoolTransaction) -> Result<(), TxInsersionError> {
        // Get the nonce chain for the contract

        match self.nonce_chains.entry(tx.contract_address()) {
            hash_map::Entry::Occupied(entry) => {

            },
            hash_map::Entry::Vacant(entry) => todo!(),
        }
        let chain = .or_insert(NonceChain::default());

        // Insert the tx in the nonce chain
        if !chain.insert(tx) {
            return Err(TxInsersionError::NonceConflict)
        }

        self.tx_queue.push(item)



        Ok(())
    }

    pub fn has_deployed_contract(&self, addr: &ContractAddress) -> bool {
        self.deployed_contracts.contains(addr)
    }

    pub fn pop_next_nonce_chain(&mut self) -> Option<NonceChain> {
        self.tx_queue.pop_first().map(|el| el.0)
    }
    pub fn re_add_nonce_chain(&mut self, chain: NonceChain) {
        self.tx_queue.insert(NonceChainByNextTimestamp(chain));
    }
}
