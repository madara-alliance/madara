//! The inner mempool does not perform validation, and is expected to be stored into a RwLock or Mutex.
//! This is the chokepoint for all insertions and popping, as such, we want to make it as fast as possible.
//! Insertion and popping should be O(log n).
//! We also really don't want to poison the lock by panicking.

use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use deployed_contracts::DeployedContracts;
use nonce_chain::{InsertedPosition, NonceChain, NonceChainNewState, ReplacedState};
use starknet_api::core::ContractAddress;
use std::{
    cmp,
    collections::{hash_map, BTreeSet, HashMap},
};

mod deployed_contracts;
mod nonce_chain;
mod tx;
pub use tx::*;

#[derive(Clone, Debug)]
struct AccountOrderedByTimestamp {
    contract_addr: ContractAddress,
    timestamp: ArrivedAtTimestamp,
}

impl PartialEq for AccountOrderedByTimestamp {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for AccountOrderedByTimestamp {}
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

#[derive(Default, Debug)]
/// Invariants:
/// - Every nonce chain in `nonce_chains` should have a one to one match with `tx_queue`.
/// - Every [`AccountTransaction::DeployAccount`] transaction should have a one to one match with `deployed_contracts`.
/// - See [`NonceChain`] invariants.
pub struct MempoolInner {
    /// We have one nonce chain per contract address.
    nonce_chains: HashMap<ContractAddress, NonceChain>,
    /// FCFS queue.
    tx_queue: BTreeSet<AccountOrderedByTimestamp>,
    deployed_contracts: DeployedContracts,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum TxInsersionError {
    #[error("A transaction with this nonce already exists in the transaction pool")]
    NonceConflict,
    #[error("A transaction with this hash already exists in the transaction pool")]
    DuplicateTxn,
}

impl MempoolInner {
    #[cfg(test)]
    pub fn check_invariants(&self) {
        self.nonce_chains.values().for_each(NonceChain::check_invariants);
        let mut tx_queue = self.tx_queue.clone();
        for (k, v) in &self.nonce_chains {
            assert!(tx_queue.remove(&AccountOrderedByTimestamp { contract_addr: *k, timestamp: v.front_arrived_at }))
        }
        assert!(tx_queue.is_empty());
        let mut deployed_contracts = self.deployed_contracts.clone();
        for (contract, _) in self.nonce_chains.values().flat_map(|chain| &chain.transactions) {
            if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &contract.0.tx {
                deployed_contracts.decrement(tx.contract_address)
            }
        }
        assert!(deployed_contracts.is_empty(), "remaining deployed_contracts: {deployed_contracts:?}");
    }

    /// When `force` is `true`, this function should never return any error.
    pub fn insert_tx(&mut self, mempool_tx: MempoolTransaction, force: bool) -> Result<(), TxInsersionError> {
        let contract_addr = mempool_tx.contract_address();
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
                let (position, is_replaced) = match entry.get_mut().insert(mempool_tx, force) {
                    Ok(position) => position,
                    Err(nonce_collision_or_duplicate_hash) => {
                        if force {
                            panic!("Force add should never error")
                        }
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

        if is_replaced != ReplacedState::Replaced {
            if let Some(contract_address) = &deployed_contract_address {
                self.deployed_contracts.increment(*contract_address)
            }
        }

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
        if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &mempool_tx.tx {
            self.deployed_contracts.decrement(tx.contract_address);
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
    use super::*;
    use blockifier::{
        execution::contract_class::ClassInfo,
        test_utils::{contracts::FeatureContract, CairoVersion},
        transaction::{transaction_execution::Transaction, transaction_types::TransactionType},
    };
    use mc_exec::execution::TxInfo;
    use mp_convert::ToFelt;
    use proptest::prelude::*;
    use proptest_derive::Arbitrary;
    use starknet_api::{
        core::{calculate_contract_address, ChainId, Nonce},
        data_availability::DataAvailabilityMode,
        transaction::{
            ContractAddressSalt, DeclareTransactionV3, DeployAccountTransactionV3, InvokeTransactionV3, Resource,
            ResourceBounds, ResourceBoundsMapping, TransactionHash, TransactionHasher, TransactionVersion,
        },
    };
    use starknet_types_core::felt::Felt;

    use blockifier::abi::abi_utils::selector_from_name;
    use starknet_api::transaction::Fee;
    use std::{
        collections::HashSet,
        fmt,
        time::{Duration, SystemTime},
    };

    lazy_static::lazy_static! {
        static ref DUMMY_CLASS: ClassInfo = {
            let dummy_contract_class = FeatureContract::TestContract(CairoVersion::Cairo1);
            ClassInfo::new(&dummy_contract_class.get_class(), 100, 100).unwrap()
        };
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
                L1Handler,
            }

            <(TxTy, u8, u8, u8, bool)>::arbitrary()
                .prop_map(|(ty, arrived_at, contract_address, nonce, force)| {
                    let arrived_at = SystemTime::UNIX_EPOCH + Duration::from_millis(arrived_at.into());
                    let contract_addr = ContractAddress::try_from(Felt::from(contract_address)).unwrap();
                    let nonce = Nonce(Felt::from(nonce));

                    let resource_bounds = ResourceBoundsMapping(
                        [
                            (Resource::L1Gas, ResourceBounds { max_amount: 5, max_price_per_unit: 5 }),
                            (Resource::L2Gas, ResourceBounds { max_amount: 5, max_price_per_unit: 5 }),
                        ]
                        .into(),
                    );

                    let tx = match ty {
                        TxTy::Declare => starknet_api::transaction::Transaction::Declare(
                            starknet_api::transaction::DeclareTransaction::V3(DeclareTransactionV3 {
                                resource_bounds,
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
                        ),
                        TxTy::DeployAccount => starknet_api::transaction::Transaction::DeployAccount(
                            starknet_api::transaction::DeployAccountTransaction::V3(DeployAccountTransactionV3 {
                                resource_bounds,
                                tip: Default::default(),
                                signature: Default::default(),
                                nonce,
                                class_hash: Default::default(),
                                nonce_data_availability_mode: DataAvailabilityMode::L1,
                                fee_data_availability_mode: DataAvailabilityMode::L1,
                                paymaster_data: Default::default(),
                                contract_address_salt: ContractAddressSalt(contract_addr.to_felt()),
                                constructor_calldata: Default::default(),
                            }),
                        ),
                        TxTy::InvokeFunction => starknet_api::transaction::Transaction::Invoke(
                            starknet_api::transaction::InvokeTransaction::V3(InvokeTransactionV3 {
                                resource_bounds,
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
                        ),
                        // TODO: maybe update the values?
                        TxTy::L1Handler => starknet_api::transaction::Transaction::L1Handler(
                            starknet_api::transaction::L1HandlerTransaction {
                                version: TransactionVersion::ZERO,
                                nonce,
                                contract_address: contract_addr,
                                entry_point_selector: selector_from_name("l1_handler_set_value"),
                                calldata: Default::default(),
                            },
                        ),
                    };

                    let deployed = if let starknet_api::transaction::Transaction::DeployAccount(tx) = &tx {
                        Some(
                            calculate_contract_address(
                                tx.contract_address_salt(),
                                Default::default(),
                                &Default::default(),
                                Default::default(),
                            )
                            .unwrap(),
                        )
                    } else {
                        None
                    };

                    // providing dummy l1 gas for now
                    let l1_gas_paid = match &tx {
                        starknet_api::transaction::Transaction::L1Handler(_) => Some(Fee(1)),
                        _ => None,
                    };

                    let tx_hash = tx.calculate_transaction_hash(&ChainId::Mainnet, &TransactionVersion::THREE).unwrap();

                    let tx =
                        Transaction::from_api(tx, tx_hash, Some(DUMMY_CLASS.clone()), l1_gas_paid, deployed, false)
                            .unwrap();

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
            tracing::debug!("\n\n\n\n\nCase: {:#?}", self);
            let mut mempool = MempoolInner::default();
            mempool.check_invariants();

            let mut inserted = HashSet::new();
            let mut inserted_contract_nonce_pairs = HashSet::new();
            let mut new_contracts = HashSet::new();

            let handle_pop = |res: Option<MempoolTransaction>,
                              inserted: &mut HashSet<TransactionHash>,
                              inserted_contract_nonce_pairs: &mut HashSet<(Nonce, ContractAddress)>,
                              new_contracts: &mut HashSet<ContractAddress>| {
                if let Some(res) = &res {
                    let removed = inserted.remove(&res.tx_hash());
                    assert!(removed);
                    let removed = inserted_contract_nonce_pairs.remove(&(res.nonce(), res.contract_address()));
                    assert!(removed);

                    if res.tx.tx_type() == TransactionType::DeployAccount {
                        let _removed = new_contracts.remove(&res.contract_address());
                        // there can be multiple deploy_account txs.
                        // assert!(removed)
                    }
                } else {
                    assert!(inserted.is_empty())
                }
                tracing::trace!("Popped {:?}", res.map(|el| Insert(el, false)));
            };

            for op in &self.0 {
                match op {
                    Operation::Insert(insert) => {
                        let force = insert.1;
                        tracing::trace!("Insert {:?}", insert);
                        let res = mempool.insert_tx(insert.0.clone(), insert.1);

                        let expected = if !force
                            && inserted_contract_nonce_pairs.contains(&(insert.0.nonce(), insert.0.contract_address()))
                        {
                            if inserted.contains(&insert.0.tx_hash()) {
                                Err(TxInsersionError::DuplicateTxn)
                            } else {
                                Err(TxInsersionError::NonceConflict)
                            }
                        } else {
                            Ok(())
                        };

                        assert_eq!(expected, res);

                        if expected.is_ok() {
                            if insert.0.tx.tx_type() == TransactionType::DeployAccount {
                                new_contracts.insert(insert.0.contract_address());
                            }
                            inserted.insert(insert.0.tx_hash());
                            inserted_contract_nonce_pairs.insert((insert.0.nonce(), insert.0.contract_address()));
                        }

                        tracing::trace!("Result {:?}", res);
                    }
                    Operation::Pop => {
                        tracing::trace!("Pop");
                        let res = mempool.pop_next();
                        handle_pop(res, &mut inserted, &mut inserted_contract_nonce_pairs, &mut new_contracts);
                    }
                }
                tracing::trace!("State: {mempool:#?}");
                mempool.check_invariants();
            }

            loop {
                tracing::trace!("Pop");
                let Some(res) = mempool.pop_next() else { break };
                handle_pop(Some(res), &mut inserted, &mut inserted_contract_nonce_pairs, &mut new_contracts);
                mempool.check_invariants();
            }
            assert!(inserted.is_empty());
            assert!(inserted_contract_nonce_pairs.is_empty());
            assert!(new_contracts.is_empty());
            tracing::trace!("Done :)");
        }
    }

    proptest::proptest! {
        #![proptest_config(ProptestConfig::with_cases(5))] // comment this when developing, this is mostly for faster ci & whole workspace `cargo test`
        #[test]
        fn proptest_mempool(pb in any::<MempoolInvariantsProblem>()) {
            let _ = env_logger::builder().is_test(true).try_init();
            tracing::log::set_max_level(tracing::log::LevelFilter::Trace);
            pb.check();
        }
    }
}
