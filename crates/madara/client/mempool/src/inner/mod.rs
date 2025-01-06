//! The inner mempool does not perform validation, and is expected to be stored into a RwLock or Mutex.
//! This is the chokepoint for all insertions and popping, as such, we want to make it as fast as possible.
//! Insertion and popping should be O(log n).
//! We also really don't want to poison the lock by panicking.

use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use deployed_contracts::DeployedContracts;
use mc_db::mempool_db::NonceReadiness;
use mp_convert::ToFelt;
use starknet_api::core::{ContractAddress, Nonce};
use starknet_types_core::felt::Felt;
use std::collections::{btree_map, hash_map, BTreeMap, BTreeSet, HashMap};

mod deployed_contracts;
mod intent;
mod limits;
mod nonce_mapping;
mod proptest;
mod tx;

pub(crate) use intent::*;
pub use limits::*;
pub use nonce_mapping::*;
pub use tx::*;

#[cfg(test)]
use crate::CheckInvariants;

#[derive(Debug)]
/// A struct responsible for the rapid ordering and disposal of transactions by
/// their [readiness] and time of arrival.
///
/// # Intent Queues:
///
/// FCFS queues. We use a [BTreeSet] to maintain logarithmic complexity and high
/// performance with low reordering of the memory even in the case of very high
/// transaction throughput.
///
/// These do not actually store transactions but the *intent* and the *order* of
/// these transaction being added to the [Mempool]. We *intend* to execute a
/// transaction from a given contract address, stored in each queue, based on
/// its readiness and order of arrival. A transaction is deemed ready if its
/// [Nonce] directly follows the previous [Nonce] used by that contract. This is
/// retrieved from the database before the transaction is added to the [Mempool].
/// This means that transactions which are not ready (these are [pending]
/// transactions) will stay waiting for the required transactions to  be
/// processed before they are marked as [ready] themselves.
///
/// # Updating Transaction Intent
///
/// Transaction intents in each queue are updated as follows:
///
/// - When [pop_next] is called, the next [ready] intent (if any are available)
///   is popped from its queue.
///
/// - The information contained in that intent is used to retrieve the
///   [MempoolTransaction] associated with it, which is stored inside a
///   [NonceTxMapping], inside [nonce_mapping].
///
/// - Once this is done, [MempoolInner] will try and pop the [pending] intent
///   with the next nonce at the same contract address. This will then be
///   converted into a [ready] intent and added to the relevant queue. Note that
///   it is not possible for this to result in a nonce conflict with the intent
///   read queue as those are checked when inserting transactions with
///   [insert_tx] and [NonceTxMapping::insert].
///
/// # Invariants
///
/// The inner mempool adheres to the following invariants:
///
/// - every [MempoolTransaction] mapping in [nonce_mapping] should have a
///   one-to-one match with an entry in either [tx_intent_queue_ready] or
///   [tx_intent_queue_pending].
///
/// - every [Felt] key in [nonce_mapping] should have a one-to-one match with
///   the contract address of an entry in either [tx_intent_queue_ready] or
///   [tx_intent_queue_pending].
///
/// - Every [`AccountTransaction::DeployAccount`] transaction should have a one
///   to one match with [deployed_contracts].
///
/// - The invariants of [TransactionIntentReady] and [TransactionIntentPending]
///   must be respected.
///
/// These invariants can be checked by calling [check_invariants] in a test
/// environment.
///
/// [readiness]: intent
/// [Mempool]: super::Mempool
/// [pending]: TransactionIntentPending
/// [ready]: TransactionIntentReady
/// [pop_next]: Self::pop_next
/// [nonce_mapping]: Self::nonce_mapping
/// [insert_tx]: Self::insert_tx
/// [tx_intent_queue_ready]: Self::tx_intent_queue_ready
/// [tx_intent_queue_pending]: Self::tx_intent_queue_pending
/// [deployed_contracts]: Self::deployed_contracts
/// [check_invariants]: Self::check_invariants
pub(crate) struct MempoolInner {
    /// We have one [Nonce] to  [MempoolTransaction] mapping per contract
    /// address.
    pub(crate) nonce_mapping: HashMap<Felt, NonceTxMapping>,
    /// FCFS queue of all [ready] intents.
    ///
    /// [ready]: TransactionIntentReady
    pub(crate) tx_intent_queue_ready: BTreeSet<TransactionIntentReady>,
    /// FCFS queue of all [pending] intents.
    ///
    /// [pending]: TransactionIntentPending
    pub(crate) tx_intent_queue_pending: BTreeMap<TransactionIntentPending, ()>,
    /// A count of all deployed contract declared so far.
    deployed_contracts: DeployedContracts,
    /// Constraints on the number of transactions allowed in the [Mempool]
    ///
    /// [Mempool]: super::Mempool
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

#[cfg(test)]
impl CheckInvariants for MempoolInner {
    fn check_invariants(&self) {
        let mut tx_counts = HashMap::<Felt, usize>::default();
        for intent in self.tx_intent_queue_ready.iter() {
            // TODO: check the nonce against the db to make sure this intent is
            // really ready
            intent.check_invariants();

            let nonce_mapping = self
                .nonce_mapping
                .get(&intent.contract_address)
                .unwrap_or_else(|| panic!("Missing nonce mapping for contract address {}", &intent.contract_address));

            let mempool_tx =
                nonce_mapping.transactions.get(&Nonce(intent.nonce)).expect("Missing nonce mapping for intent");

            assert_eq!(mempool_tx.nonce, Nonce(intent.nonce));
            assert_eq!(mempool_tx.nonce_next, Nonce(intent.nonce_next));
            assert_eq!(mempool_tx.arrived_at, intent.timestamp);

            if matches!(mempool_tx.tx, Transaction::AccountTransaction(AccountTransaction::DeployAccount(_))) {
                // TODO: make sure the deploy contract count is good?
                assert!(
                    self.has_deployed_contract(&mempool_tx.contract_address()),
                    "Deploy account tx is not part of deployed contacts"
                );
            }

            match tx_counts.entry(intent.contract_address) {
                hash_map::Entry::Occupied(mut entry) => {
                    *entry.get_mut() += 1;
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(1);
                }
            }
        }

        for (intent, _) in self.tx_intent_queue_pending.iter() {
            // TODO: check the nonce against the db to make sure this intent is
            // really pending
            intent.check_invariants();

            let nonce_mapping = self
                .nonce_mapping
                .get(&intent.contract_address)
                .unwrap_or_else(|| panic!("Missing nonce mapping for contract address {}", &intent.contract_address));

            let mempool_tx = nonce_mapping.transactions.get(&Nonce(intent.nonce)).unwrap_or_else(|| {
                panic!(
                    "Missing nonce mapping for intent: required {}, available {:?}",
                    intent.nonce,
                    nonce_mapping.transactions.keys()
                )
            });

            assert_eq!(mempool_tx.nonce, Nonce(intent.nonce));
            assert_eq!(mempool_tx.nonce_next, Nonce(intent.nonce_next));
            assert_eq!(mempool_tx.arrived_at, intent.timestamp);

            if matches!(mempool_tx.tx, Transaction::AccountTransaction(AccountTransaction::DeployAccount(_))) {
                assert!(
                    self.has_deployed_contract(&mempool_tx.contract_address()),
                    "Deploy account tx is not part of deployed contacts"
                );
            }

            match tx_counts.entry(intent.contract_address) {
                hash_map::Entry::Occupied(mut entry) => {
                    *entry.get_mut() += 1;
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(1);
                }
            }
        }

        for (contract_address, nonce_mapping) in self.nonce_mapping.iter() {
            let count = tx_counts.get(contract_address).unwrap_or_else(|| {
                panic!(
                    "Extra nonce mapping at contract address {contract_address}, remaining nonces are: {:?}",
                    nonce_mapping.transactions.keys()
                )
            });

            assert_eq!(
                &nonce_mapping.transactions.len(),
                count,
                "Extra transactions in nonce mapping at contract address {contract_address}, remaining nonces are: {:?}",
                nonce_mapping.transactions.keys()
            );
        }
    }
}

impl MempoolInner {
    pub fn new(limits_config: MempoolLimits) -> Self {
        Self {
            nonce_mapping: Default::default(),
            tx_intent_queue_ready: Default::default(),
            tx_intent_queue_pending: Default::default(),
            deployed_contracts: Default::default(),
            limiter: MempoolLimiter::new(limits_config),
        }
    }

    /// When `force` is `true`, this function should never return any error.
    /// `update_limits` is `false` when the transaction has been removed from
    /// the mempool in the past without updating the limits.
    pub fn insert_tx(
        &mut self,
        mempool_tx: MempoolTransaction,
        force: bool,
        update_limits: bool,
        readiness: NonceReadiness,
    ) -> Result<(), TxInsersionError> {
        // delete age-exceeded txs from the mempool
        // todo(perf): this may want to limit this check once every few seconds
        // to avoid it being in the hot path?
        self.remove_age_exceeded_txs();

        // check limits
        let limits_for_tx = TransactionCheckedLimits::limits_for(&mempool_tx);
        if !force {
            self.limiter.check_insert_limits(&limits_for_tx)?;
        }

        let contract_address = mempool_tx.contract_address().to_felt();
        let arrived_at = mempool_tx.arrived_at;
        let deployed_contract_address =
            if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &mempool_tx.tx {
                Some(tx.contract_address)
            } else {
                None
            };

        // Inserts the transaction into the nonce tx mapping for the current
        // contract
        match self.nonce_mapping.entry(contract_address) {
            hash_map::Entry::Occupied(mut entry) => {
                // Handle nonce collision.
                let nonce_tx_mapping = entry.get_mut();
                // TODO: reword nonces to be of the Nonce type and decouple
                // nonce storage from the readiness state
                let replaced = match nonce_tx_mapping.insert(mempool_tx, Nonce(readiness.nonce()), force) {
                    Ok(replaced) => replaced,
                    Err(nonce_collision_or_duplicate_hash) => {
                        debug_assert!(!force); // Force add should never error
                        return Err(nonce_collision_or_duplicate_hash);
                    }
                };

                // Update the tx queues.
                match readiness {
                    NonceReadiness::Ready { nonce, nonce_next } => {
                        // Remove old value (if collision and force == true)
                        if let ReplacedState::Replaced { previous } = replaced {
                            let removed = self.tx_intent_queue_ready.remove(&TransactionIntentReady {
                                contract_address,
                                timestamp: previous.arrived_at,
                                nonce,
                                nonce_next,
                            });
                            debug_assert!(removed);
                            self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&previous));
                        } else if let Some(contract_address) = &deployed_contract_address {
                            self.deployed_contracts.increment(*contract_address)
                        }

                        // Insert new value. Here, we need two lookups: one for
                        // removal and one for insertion because
                        // TransactionIntentReady considers `timestamp` in its
                        // implementation of Eq.
                        let insert = self.tx_intent_queue_ready.insert(TransactionIntentReady {
                            contract_address,
                            timestamp: arrived_at,
                            nonce,
                            nonce_next,
                        });
                        debug_assert!(insert);
                    }
                    NonceReadiness::Pending { nonce, nonce_next } => {
                        // Insert new value. This takes advantage of the fact
                        // that `timestamp` is not considered in the
                        // implementation of Eq for TransactionIntentPending to
                        // avoid a double lookup.
                        #[allow(unused)]
                        let insert_or_replace = self
                            .tx_intent_queue_pending
                            .insert(
                                TransactionIntentPending { contract_address, timestamp: arrived_at, nonce, nonce_next },
                                (),
                            )
                            .is_none();

                        #[cfg(debug_assertions)]
                        {
                            if matches!(replaced, ReplacedState::Replaced { .. }) {
                                debug_assert!(!insert_or_replace);
                            } else {
                                debug_assert!(insert_or_replace);
                            }
                        }

                        // Remove old value (if collision and force == true)
                        if let ReplacedState::Replaced { previous } = replaced {
                            self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&previous));
                        } else if let Some(contract_address) = &deployed_contract_address {
                            self.deployed_contracts.increment(*contract_address);
                        }
                    }
                };
            }
            hash_map::Entry::Vacant(entry) => {
                // Insert the new nonce tx mapping
                let nonce_tx_mapping = NonceTxMapping::new_with_first_tx(mempool_tx, Nonce(readiness.nonce()));
                entry.insert(nonce_tx_mapping);

                // Update the tx queues.
                let inserted = match readiness {
                    NonceReadiness::Ready { nonce, nonce_next } => self
                        .tx_intent_queue_ready
                        .insert(TransactionIntentReady { contract_address, timestamp: arrived_at, nonce, nonce_next }),
                    NonceReadiness::Pending { nonce, nonce_next } => self
                        .tx_intent_queue_pending
                        .insert(
                            TransactionIntentPending { contract_address, timestamp: arrived_at, nonce, nonce_next },
                            (),
                        )
                        .is_none(),
                };
                debug_assert!(inserted);

                if let Some(contract_address) = &deployed_contract_address {
                    self.deployed_contracts.increment(*contract_address)
                }
            }
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

    pub fn remove_age_exceeded_txs(&mut self) {
        // We take advantage of the fact that TransactionIntentReady is
        // ordered by timestamp, so as soon as we find a transaction which has
        // not exceeded its max age (and that transaction supports age limits)
        // we know no more transactions can be removed.
        //
        // INFO: we traverse this in reverse as the intents at the end of the
        // queue have the highest chance of pointing to transactions which have
        // exceeded their age limit. It is very unlikely that intents at the
        // front of the queue point to such transactions. If that were the case,
        // it would be handled by pop_next anyway. This should help with
        // maximizing the number of removals and keeping the mempool from being
        // congested.
        while let Some(tx_intent) = self.tx_intent_queue_ready.last() {
            let tx = self
                .nonce_mapping
                .get_mut(&tx_intent.contract_address)
                .expect("Nonce chain does not match tx queue")
                .transactions
                .get(&Nonce(tx_intent.nonce))
                .expect("Nonce chain without a tx");

            let limits = TransactionCheckedLimits::limits_for(tx);

            if self.limiter.tx_age_exceeded(&limits) {
                self.tx_intent_queue_ready.pop_last();
            } else if limits.checks_age() {
                break;
            }
        }

        // TODO: need to re-think how we remove pending intents since they
        // cannot be ordered by timestamp to allow for retrieval by the
        // previous ready intent (which cannot know the timestamp of the
        // pending transaction directly following it). Perhaps we need something
        // similar to `nonce_mapping` for pending intents, and have THOSE sorted
        // by timestamp?
        //
        // while let Some((tx_intent, _)) = self.tx_intent_queue_pending.last_key_value() {
        //     let tx = self
        //         .nonce_mapping
        //         .get_mut(&tx_intent.contract_address)
        //         .expect("Nonce chain does not match tx queue")
        //         .transactions
        //         .get(&Nonce(tx_intent.nonce))
        //         .expect("Nonce chain without a tx");
        //
        //     let limits = TransactionCheckedLimits::limits_for(tx);
        //
        //     if self.limiter.tx_age_exceeded(&limits) {
        //         self.tx_intent_queue_ready.pop_last();
        //     } else if limits.checks_age() {
        //         break;
        //     }
        // }
    }

    pub fn pop_next(&mut self) -> Option<MempoolTransaction> {
        // Pop tx queue.
        let (tx_mempool, tx_next) = loop {
            // Bubble up None if the mempool is empty.
            let tx_intent = self.tx_intent_queue_ready.pop_first()?;
            let tx_mempool = self.pop_tx_from_intent(&tx_intent);

            let limits = TransactionCheckedLimits::limits_for(&tx_mempool);
            if !self.limiter.tx_age_exceeded(&limits) {
                break (tx_mempool, tx_intent.tx_next_for_lookup());
            }

            // transaction age exceeded, remove the tx from mempool.
            self.limiter.mark_removed(&limits);
        };

        // Looks for the next transaction from the same account in the pending
        // queue and marks it as ready if found.
        // TODO: maybe this can be simplified by storing a mapping
        // contract_address -> pending intent queue, then we just need to
        // compare the nonce on the first entry and perhaps pop it. Seems like
        // a good idea.
        if let btree_map::Entry::Occupied(entry) = self.tx_intent_queue_pending.entry(tx_next) {
            let tx_ready = entry.key().ready();
            entry.remove();
            self.tx_intent_queue_ready.insert(tx_ready);
        }

        // do not update mempool limits, block prod will update it with re-add txs.
        Some(tx_mempool)
    }

    fn pop_tx_from_intent(&mut self, tx_queue_account: &TransactionIntentReady) -> MempoolTransaction {
        let nonce_tx_mapping = self
            .nonce_mapping
            .get_mut(&tx_queue_account.contract_address)
            .expect("Nonce chain does not match tx queue");

        // Get the next ready transaction from the nonce chain
        let (mempool_tx, nonce_tx_mapping_new_state) = nonce_tx_mapping.pop();
        if nonce_tx_mapping_new_state == NonceTxMappingNewState::Empty {
            let removed = self.nonce_mapping.remove(&tx_queue_account.contract_address);
            debug_assert!(removed.is_some());
        }

        // Update deployed contracts.
        if let Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) = &mempool_tx.tx {
            self.deployed_contracts.decrement(tx.contract_address);
        }

        mempool_tx
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
            // Since this is re-adding a transaction which was already popped
            // from the mempool, we can be sure it is ready
            let nonce = *tx.nonce;
            let nonce_next = *tx.nonce_next;
            self.insert_tx(tx, force, false, NonceReadiness::Ready { nonce, nonce_next })
                .expect("Force insert tx should not error");
        }
    }

    // This is called by the block production when loading the pending block
    // from db
    pub fn insert_txs(
        &mut self,
        txs: impl IntoIterator<Item = MempoolTransaction>,
        force: bool,
    ) -> Result<(), TxInsersionError> {
        for tx in txs {
            // Transactions are marked as ready as they were already included
            // into the pending block
            let nonce = *tx.nonce;
            let nonce_next = *tx.nonce_next;
            self.insert_tx(tx, force, true, NonceReadiness::Ready { nonce, nonce_next })?;
        }
        Ok(())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_empty(&self) -> bool {
        self.tx_intent_queue_ready.is_empty()
    }
}
