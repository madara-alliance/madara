//! The inner mempool does not perform validation, and is expected to be stored into a RwLock or Mutex.
//! This is the chokepoint for all insertions and popping, as such, we want to make it as fast as possible.
//! Insertion and popping should be O(log n).
//! We also really don't want to poison the lock by panicking.
//!
//! TODO: mempool size limits
//! TODO(perf): should we box the MempoolTransaction?

use crate::{clone_account_tx, contract_addr, nonce, tx_hash};
use blockifier::transaction::account_transaction::AccountTransaction;
use mp_class::ConvertedClass;
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
};
use std::{
    cmp,
    collections::{hash_map, BTreeSet, HashMap, HashSet},
    iter,
    time::SystemTime,
};

pub type ArrivedAtTimestamp = SystemTime;

#[derive(Debug)]
pub struct MempoolTransaction {
    pub tx: AccountTransaction,
    pub arrived_at: ArrivedAtTimestamp,
    pub converted_class: Option<ConvertedClass>,
}

impl Clone for MempoolTransaction {
    fn clone(&self) -> Self {
        Self {
            tx: clone_account_tx(&self.tx),
            arrived_at: self.arrived_at,
            converted_class: self.converted_class.clone(),
        }
    }
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
        self.0.nonce() == other.0.nonce()
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
pub struct NonceChain {
    transactions: BTreeSet<OrderMempoolTransactionByNonce>,
    front_arrived_at: ArrivedAtTimestamp,
    #[cfg(debug_assertions)]
    front_tx_hash: TransactionHash,
}

#[derive(Eq, PartialEq, Debug)]
pub enum InsertedPosition {
    Front { former_head_arrived_at: ArrivedAtTimestamp },
    Other,
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
            #[cfg(debug_assertions)]
            front_tx_hash: tx.tx_hash(),
            transactions: iter::once(OrderMempoolTransactionByNonce(tx)).collect(),
        }
    }

    #[cfg(test)]
    pub fn check_invariants(&self) {
        debug_assert!(!self.transactions.is_empty());
        let front = self.transactions.first().unwrap();
        debug_assert_eq!(front.0.tx_hash(), self.front_tx_hash);
        debug_assert_eq!(front.0.arrived_at, self.front_arrived_at);
    }

    /// Returns where in the chain it was inserted.
    /// When `force` is `true`, this function should never return any error.
    pub fn insert(
        &mut self,
        mempool_tx: MempoolTransaction,
        force: bool,
    ) -> Result<InsertedPosition, TxInsersionError> {
        let position = if self.front_arrived_at > mempool_tx.arrived_at {
            // We are inserting at the front here
            let former_head_arrived_at = self.front_arrived_at;
            self.front_arrived_at = mempool_tx.arrived_at;
            #[cfg(debug_assertions)]
            {
                self.front_tx_hash = mempool_tx.tx_hash();
            }
            InsertedPosition::Front { former_head_arrived_at }
        } else {
            InsertedPosition::Other
        };

        #[cfg(debug_assertions)] // unknown field `front_tx_hash` in release if debug_assert_eq is used
        assert_eq!(self.transactions.first().expect("Getting the first tx").0.tx_hash(), self.front_tx_hash);

        if force {
            self.transactions.replace(OrderMempoolTransactionByNonce(mempool_tx));
        } else if !self.transactions.insert(OrderMempoolTransactionByNonce(mempool_tx)) {
            return Err(TxInsersionError::NonceConflict);
        }

        Ok(position)
    }

    pub fn pop(&mut self) -> (MempoolTransaction, NonceChainNewState) {
        // TODO(perf): avoid double lookup
        let tx = self.transactions.pop_first().expect("Nonce chain should not be empty");
        if let Some(new_front) = self.transactions.first() {
            self.front_arrived_at = new_front.0.arrived_at;
            #[cfg(debug_assertions)]
            {
                self.front_tx_hash = new_front.0.tx_hash();
            }
            (tx.0, NonceChainNewState::NotEmpty)
        } else {
            (tx.0, NonceChainNewState::Empty)
        }
    }
}

#[derive(Clone)]
struct AccountOrderedByTimestamp {
    contract_addr: ContractAddress,
    timestamp: ArrivedAtTimestamp,
}

impl PartialEq for AccountOrderedByTimestamp {
    fn eq(&self, other: &Self) -> bool {
        // Important: Contract addr here, not timestamp.
        // There can be timestamp collisions.
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
        Some(self.cmp(other))
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
    #[cfg(test)]
    pub fn check_invariants(&self) {
        self.nonce_chains.values().for_each(NonceChain::check_invariants);
        let mut tx_queue = self.tx_queue.clone();
        for (k, v) in &self.nonce_chains {
            debug_assert!(
                tx_queue.remove(&AccountOrderedByTimestamp { contract_addr: *k, timestamp: v.front_arrived_at })
            )
        }
        debug_assert!(tx_queue.is_empty());
        let mut deployed_contracts = self.deployed_contracts.clone();
        for contract in self.nonce_chains.values().flat_map(|chain| &chain.transactions) {
            if let AccountTransaction::DeployAccount(tx) = &contract.0.tx {
                debug_assert!(deployed_contracts.remove(&tx.contract_address))
            };
        }
        debug_assert!(deployed_contracts.is_empty());
    }

    /// When `force` is `true`, this function should never return any error.
    pub fn insert_tx(&mut self, mempool_tx: MempoolTransaction, force: bool) -> Result<(), TxInsersionError> {
        // Get the nonce chain for the contract

        let contract_addr = mempool_tx.contract_address();
        let arrived_at = mempool_tx.arrived_at;

        let deployed_contract_address =
            if let AccountTransaction::DeployAccount(tx) = &mempool_tx.tx { Some(tx.contract_address) } else { None };

        if let Some(contract_address) = &deployed_contract_address {
            if !self.deployed_contracts.insert(*contract_address) && !force {
                return Err(TxInsersionError::AccountAlreadyDeployed);
            }
        }

        match self.nonce_chains.entry(contract_addr) {
            hash_map::Entry::Occupied(mut entry) => {
                // Handle nonce collision.
                let position = match entry.get_mut().insert(mempool_tx, force) {
                    Ok(position) => position,
                    Err(_nonce_collision) => {
                        if force {
                            panic!("Force add should never error")
                        }
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

    pub fn pop_next(&mut self) -> Option<MempoolTransaction> {
        // Pop tx queue.
        let tx_queue_account = self.tx_queue.pop_first()?; // Bubble up None if the mempool is empty.

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
        if let AccountTransaction::DeployAccount(tx) = &mempool_tx.tx {
            let removed = self.deployed_contracts.remove(&tx.contract_address);
            debug_assert!(removed);
        }

        Some(mempool_tx)
    }

    pub fn pop_next_chunk(&mut self, dest: &mut impl Extend<MempoolTransaction>, n: usize) {
        dest.extend((0..n).map_while(|_| self.pop_next()))
    }

    pub fn re_add_txs(&mut self, txs: impl IntoIterator<Item = MempoolTransaction>) {
        for tx in txs {
            let force = true;
            self.insert_tx(tx, force).expect("Force insert tx should not error");
        }
    }
}

#[cfg(test)]
mod tests {
    use bitvec::{order::Msb0, vec::BitVec};
    use blockifier::{
        execution::contract_class::ClassInfo,
        test_utils::{contracts::FeatureContract, CairoVersion},
        transaction::transactions::{DeclareTransaction, InvokeTransaction},
    };
    use proptest::prelude::*;
    use proptest_derive::Arbitrary;
    use starknet_api::{
        data_availability::DataAvailabilityMode,
        transaction::{DeclareTransactionV3, InvokeTransactionV3},
    };
    use starknet_types_core::felt::Felt;

    use super::*;
    use std::fmt;

    #[derive(PartialEq, Eq, Hash)]
    struct AFelt(Felt);
    impl fmt::Debug for AFelt {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:#x}", self.0)
        }
    }
    impl Arbitrary for AFelt {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            <[bool; 250]>::arbitrary()
                .prop_map(|arr| arr.into_iter().collect::<BitVec<u8, Msb0>>())
                .prop_map(|vec| Felt::from_bytes_be(vec.as_raw_slice().try_into().unwrap()))
                .prop_map(Self)
                .boxed()
        }
    }

    struct Insert(MempoolTransaction, /* force */ bool);
    impl fmt::Debug for Insert {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "Insert(ty={:?},arrived_at={:?},tx_hash={:?},contract_address={:?},nonce={:?},force={:?})",
                self.0.tx.tx_type(),
                self.0.arrived_at,
                self.0.tx_hash(),
                self.0.contract_address(),
                self.0.nonce(),
                self.1,
            )
        }
    }
    impl Arbitrary for Insert {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            #[derive(Debug, Arbitrary)]
            enum TxTy {
                Declare,
                DeployAccount,
                InvokeFunction,
            }

            <(TxTy, SystemTime, AFelt, AFelt, u64, bool)>::arbitrary()
                .prop_map(|(ty, arrived_at, tx_hash, contract_address, nonce, force)| {
                    let tx_hash = TransactionHash(tx_hash.0);
                    let contract_addr = ContractAddress::try_from(contract_address.0).unwrap();
                    let nonce = Nonce(Felt::from(nonce));

                    let dummy_contract_class = FeatureContract::TestContract(CairoVersion::Cairo1);
                    let dummy_class_info = ClassInfo::new(&dummy_contract_class.get_class(), 100, 100).unwrap();

                    let tx = match ty {
                        TxTy::Declare => AccountTransaction::Declare(
                            DeclareTransaction::new(
                                starknet_api::transaction::DeclareTransaction::V3(DeclareTransactionV3 {
                                    resource_bounds: Default::default(),
                                    tip: Default::default(),
                                    signature: Default::default(),
                                    nonce,
                                    class_hash: Default::default(),
                                    compiled_class_hash: Default::default(),
                                    sender_address: contract_addr,
                                    nonce_data_availability_mode: DataAvailabilityMode::L1,
                                    fee_data_availability_mode: DataAvailabilityMode::L1,
                                    paymaster_data: Default::default(),
                                    account_deployment_data: Default::default(),
                                }),
                                tx_hash,
                                dummy_class_info,
                            )
                            .unwrap(),
                        ),
                        TxTy::DeployAccount => AccountTransaction::Declare(
                            DeclareTransaction::new(
                                starknet_api::transaction::DeclareTransaction::V3(DeclareTransactionV3 {
                                    resource_bounds: Default::default(),
                                    tip: Default::default(),
                                    signature: Default::default(),
                                    nonce,
                                    class_hash: Default::default(),
                                    compiled_class_hash: Default::default(),
                                    sender_address: contract_addr,
                                    nonce_data_availability_mode: DataAvailabilityMode::L1,
                                    fee_data_availability_mode: DataAvailabilityMode::L1,
                                    paymaster_data: Default::default(),
                                    account_deployment_data: Default::default(),
                                }),
                                tx_hash,
                                dummy_class_info,
                            )
                            .unwrap(),
                        ),
                        TxTy::InvokeFunction => AccountTransaction::Invoke(InvokeTransaction::new(
                            starknet_api::transaction::InvokeTransaction::V3(InvokeTransactionV3 {
                                resource_bounds: Default::default(),
                                tip: Default::default(),
                                signature: Default::default(),
                                nonce,
                                sender_address: contract_addr,
                                calldata: Default::default(),
                                nonce_data_availability_mode: DataAvailabilityMode::L1,
                                fee_data_availability_mode: DataAvailabilityMode::L1,
                                paymaster_data: Default::default(),
                                account_deployment_data: Default::default(),
                            }),
                            tx_hash,
                        )),
                    };

                    Insert(MempoolTransaction { tx, arrived_at, converted_class: None }, force)
                })
                .boxed()
        }
    }

    #[derive(Debug, Arbitrary)]
    enum Operation {
        Insert(Insert),
        Pop,
    }

    #[derive(Debug, Arbitrary)]
    struct MempoolInvariantsProblem(Vec<Operation>);
    impl MempoolInvariantsProblem {
        fn check(&self) {
            let mut mempool = MempoolInner::default();
            mempool.check_invariants();

            let mut inserted = HashSet::new();

            for op in &self.0 {
                match op {
                    Operation::Insert(insert) => {
                        log::trace!("Insert {:?}", insert);
                        let res = mempool.insert_tx(insert.0.clone(), insert.1);
                        log::trace!("Result {:?}", res);
                        inserted.insert(insert.0.tx_hash());
                    }
                    Operation::Pop => {
                        log::trace!("Pop");
                        let res = mempool.pop_next();
                        if let Some(res) = &res {
                            inserted.remove(&res.tx_hash());
                        }
                        log::trace!("Popped {:?}", res.map(|el| Insert(el, false)));
                    }
                }
                mempool.check_invariants();
            }

            loop {
                log::trace!("Pop");
                let Some(res) = mempool.pop_next() else { break };
                inserted.remove(&res.tx_hash());
                log::trace!("Popped {:?}", Insert(res, false));
                mempool.check_invariants();
            }
            assert!(inserted.is_empty());
            log::trace!("Done :)");
        }
    }

    proptest::proptest! {
        #![proptest_config(ProptestConfig::with_cases(5))] // comment this when developing, this is mostly for faster ci & whole workspace `cargo test`
        #[test]
        fn proptest_mempool(pb in any::<MempoolInvariantsProblem>()) {
            let _ = env_logger::builder().is_test(true).try_init();
            log::set_max_level(log::LevelFilter::Trace);
            pb.check();
        }
    }
}
