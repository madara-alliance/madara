//! The inner mempool does not perform validation, and is expected to be stored into a RwLock or Mutex.
//! This is the chokepoint for all insertions and popping, as such, we want to make it as fast as possible.
//! Insertion and popping should be O(log n).
//! We also really don't want to poison the lock by panicking.

use deployed_contracts::DeployedContracts;
use mc_db::mempool_db::{NonceInfo, NonceStatus};
use mc_exec::execution::TxInfo;
use mp_convert::ToFelt;
use starknet_api::transaction::TransactionHash;
use starknet_api::{
    core::{ContractAddress, Nonce},
    executable_transaction::TransactionType,
};
use starknet_types_core::felt::Felt;
use std::collections::{btree_map, hash_map, BTreeMap, BTreeSet, HashMap, HashSet};

mod deployed_contracts;
mod intent;
mod limits;
mod nonce_mapping;
mod property_testing;
mod tx;

pub(crate) use intent::*;
pub use limits::*;
pub use nonce_mapping::*;
pub use tx::*;

#[cfg(any(test, feature = "testing"))]
use crate::CheckInvariants;

/// A struct responsible for the rapid ordering and disposal of transactions by
/// their [readiness] and time of arrival.
///
/// # Intent Queues:
///
/// These do not actually store transactions but the *intent* and the *order* of
/// these transaction being added to the [Mempool]. We *intend* to execute a
/// transaction from a given contract address, based on its readiness and order
/// of arrival. A transaction is deemed ready if its [Nonce] directly follows
/// the previous [Nonce] used by that contract. This is retrieved from the
/// database before the transaction is added to the inner mempool. This means
/// that transactions which are not ready (these are [pending] transactions)
/// will remain waiting for the required transactions to be processed before
/// they are marked as [ready] themselves.
///
/// ## [Ready]
///
/// FIFO queue. We use a [BTreeSet] to maintain logarithmic complexity and high
/// performance with low reordering of the memory even in the case of very high
/// transaction throughput.
///
/// ## [Pending]
///
/// FIFO queue. The queue has an entry per contract address in the mempool, with
/// each contract address having a [BTreeMap] queue of its transactions mapped
/// to it. We do this to have access to [BTreeMap::entry] which avoids a double
/// lookup in [pop_next] when moving pending intents to the ready queue. Intents
/// in this queue are ordered by their [Nonce].
///
/// While this is handy to retrieve the tx with the next nonce for a particular
/// contract, it is a performance bottleneck when removing age exceeded pending
/// transaction. For this reason, we keep a [separate ordering] of all pending
/// transactions, sorted by their time of arrival.
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
/// - Once this is done, we retrieve the pending queue for that contract address
///   in [tx_intent_queue_pending_by_nonce] and check if the next [pending]
///   intent has the right nonce. If so, we pop it, convert it to [ready] and
///   add it to [tx_intent_queue_ready]. If this was the last element in that
///   queue, we remove the mapping for that contract address in
///   [tx_intent_queue_pending_by_nonce].
///
/// - Finally, we update [tx_intent_queue_pending_by_timestamp] to reflect the
///   changes in [tx_intent_queue_pending_by_nonce]
///
/// # Emptying the [Mempool]
///
/// Currently, the mempool is emptied on each call to [insert_tx] based on the
/// age of a transaction. There are several issues with that.
///
/// 1. First of all, we might want to limit this check to once every few seconds
///    for performance reasons.
///
/// 2. We currently do not empty the mempool in case of congestion. This is
///    complicated because we would need to be able to differentiate between
///    declare and non-declare transactions in the mempool (declare txs have
///    their own congestion limit). This can be done by adding another readiness
///    and pending queue which are both reserved to declare transactions, but I
///    am done with refactoring for the moment and I don't even know if this
///    would be a good idea. FIXME
///
/// # Invariants
///
/// The inner mempool adheres to the following invariants:
///
/// - every [MempoolTransaction] mapping in [nonce_mapping] should have a
///   one-to-one match with an entry in either [tx_intent_queue_ready] or
///   [tx_intent_queue_pending_by_nonce].
///
/// - every [Felt] key in [nonce_mapping] should have a one-to-one match with
///   the contract address of an entry in either [tx_intent_queue_ready] or
///   [tx_intent_queue_pending_by_nonce].
///
/// - Every [`AccountTransaction::DeployAccount`] transaction should have a one
///   to one match with [deployed_contracts].
///
/// - Every intent in [tx_intent_queue_pending_by_nonce] should have a one-to-one
///   mapping with [tx_intent_queue_pending_by_timestamp].
///
/// - The invariants of [TransactionIntentReady], [TransactionIntentPendingByNonce]
///   and [TransactionIntentPendingByTimestamp] must be respected.
///
/// These invariants can be checked by calling [check_invariants] in a test
/// environment.
///
/// [Nonce]: starknet_api::core::Nonce
/// [BTreeMap]: std::collections::BTreeMap
/// [BTreeMap::entry]: std::collections::BTreeMap::entry
/// [readiness]: intent
/// [Ready]: Self::tx_intent_queue_ready
/// [Pending]: Self::tx_intent_queue_pending_by_nonce
/// [Mempool]: super::Mempool
/// [pending]: TransactionIntentPending
/// [ready]: TransactionIntentReady
/// [pop_next]: Self::pop_next
/// [nonce_mapping]: Self::nonce_mapping
/// [insert_tx]: Self::insert_tx
/// [tx_intent_queue_ready]: Self::tx_intent_queue_ready
/// [tx_intent_queue_pending_by_nonce]: Self::tx_intent_queue_pending_by_nonce
/// [tx_intent_queue_pending_by_timestamp]: Self::tx_intent_queue_pending_by_timestamp
/// [deployed_contracts]: Self::deployed_contracts
/// [check_invariants]: Self::check_invariants
/// [separate ordering]: Self::tx_intent_queue_pending_by_timestamp
#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(Clone))]
pub struct MempoolInner {
    /// We have one [Nonce] to [MempoolTransaction] mapping per contract
    /// address.
    ///
    /// [Nonce]: starknet_api::core::Nonce
    // TODO: this can be replace with a hasmap with a tupple key
    pub nonce_mapping: HashMap<Felt, NonceTxMapping>,
    /// FIFO queue of all [ready] intents.
    ///
    /// [ready]: TransactionIntentReady
    pub(crate) tx_intent_queue_ready: BTreeSet<TransactionIntentReady>,
    /// FIFO queue of all [pending] intents, sorted by their [Nonce].
    ///
    /// [pending]: TransactionIntentPendingByNonce
    // TODO: can remove contract_address from TransactionIntentPendingByNonce
    pub(crate) tx_intent_queue_pending_by_nonce: HashMap<Felt, BTreeMap<TransactionIntentPendingByNonce, ()>>,
    /// FIFO queue of all [pending] intents, sorted by their time of arrival.
    ///
    /// This is required for the rapid removal of age-exceeded txs in
    /// [remove_age_exceeded_txs] and must be kept in sync with
    /// [tx_intent_queue_pending_by_nonce].
    ///
    /// [pending]: TransactionIntentPendingByTimestamp
    /// [remove_age_exceeded_txs]: Self::remove_age_exceeded_txs
    /// [tx_intent_queue_pending_by_nonce]: Self::tx_intent_queue_pending_by_nonce
    // TODO: can remove nonce_next from TransactionIntentPendingByTimestamp
    pub(crate) tx_intent_queue_pending_by_timestamp: BTreeSet<TransactionIntentPendingByTimestamp>,
    /// List of all new deployed contracts currently in the mempool.
    pub(crate) deployed_contracts: DeployedContracts,
    /// Constraints on the number of transactions allowed in the [Mempool]
    ///
    /// [Mempool]: super::Mempool
    limiter: MempoolLimiter,

    /// Keeps track of transaction which are currently in the inner mempool by their hash
    tx_received: HashSet<TransactionHash>,

    /// This is just a helper field to use during tests to get the current nonce
    /// of a contract as known by the [MempoolInner].
    #[cfg(any(test, feature = "testing"))]
    nonce_cache_inner: HashMap<ContractAddress, Nonce>,
}

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum TxInsertionError {
    #[error("A transaction with this nonce already exists in the transaction pool")]
    NonceConflict,
    #[error("A transaction with this hash already exists in the transaction pool")]
    DuplicateTxn,
    #[error(transparent)]
    Limit(#[from] MempoolLimitReached),
}

#[cfg(any(test, feature = "testing"))]
impl CheckInvariants for MempoolInner {
    fn check_invariants(&self) {
        // tx_intent_queue_ready can only contain one tx of every contract
        let mut tx_counts = HashMap::<Felt, usize>::default();
        let mut deployed_count = 0;
        for intent in self.tx_intent_queue_ready.iter() {
            intent.check_invariants();

            let nonce_mapping = self
                .nonce_mapping
                .get(&intent.contract_address)
                .unwrap_or_else(|| panic!("Missing nonce mapping for contract address {}", &intent.contract_address));

            let mempool_tx =
                nonce_mapping.transactions.get(&intent.nonce).expect("Missing nonce mapping for ready intent");

            assert_eq!(mempool_tx.nonce, intent.nonce);
            assert_eq!(mempool_tx.nonce_next, intent.nonce_next);
            assert_eq!(mempool_tx.arrived_at, intent.timestamp);

            // DeployAccount
            if let Some(contract_address) = &mempool_tx.tx.deployed_contract_address() {
                assert!(
                    self.has_deployed_contract(contract_address),
                    "Ready deploy account tx from sender {contract_address:x?} is not part of deployed contacts"
                );
                deployed_count += 1;
            }

            *tx_counts.entry(intent.contract_address).or_insert(0) += 1;
        }

        let mut count = 0;
        for (contract_address, queue) in self.tx_intent_queue_pending_by_nonce.iter() {
            assert!(!queue.is_empty());

            for intent in queue.keys() {
                let intent_pending_by_timestamp = intent.by_timestamp();
                self.tx_intent_queue_pending_by_timestamp.get(&intent_pending_by_timestamp).unwrap_or_else(|| {
                    panic!(
                        "Missing pending intent by timestamp: {intent_pending_by_timestamp:#?}, available: {:#?}",
                        self.tx_intent_queue_pending_by_timestamp
                    )
                });
                count += 1;

                intent.check_invariants();
                assert_eq!(&intent.contract_address, contract_address);

                let nonce_mapping = self.nonce_mapping.get(&intent.contract_address).unwrap_or_else(|| {
                    panic!("Missing nonce mapping for contract address {}", &intent.contract_address)
                });

                let mempool_tx = nonce_mapping.transactions.get(&intent.nonce).unwrap_or_else(|| {
                    panic!(
                        "Missing nonce mapping for pending intent: required {:?}, available {:?}",
                        intent.nonce,
                        nonce_mapping.transactions.keys()
                    )
                });

                assert_eq!(mempool_tx.nonce, intent.nonce);
                assert_eq!(mempool_tx.nonce_next, intent.nonce_next);
                assert_eq!(mempool_tx.arrived_at, intent.timestamp);

                // DeployAccount
                if let Some(contract_address) = &mempool_tx.tx.deployed_contract_address() {
                    assert!(
                        self.has_deployed_contract(contract_address),
                        "Pending deploy account tx from sender {contract_address:x?} is not part of deployed contacts",
                    );
                    deployed_count += 1;
                }

                *tx_counts.entry(intent.contract_address).or_insert(0) += 1;
            }
        }

        assert_eq!(
            self.tx_intent_queue_pending_by_timestamp.len(),
            count,
            "Excess transactions by timetamp, remaining: {:#?}",
            self.tx_intent_queue_pending_by_timestamp
        );

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

        assert_eq!(
            deployed_count,
            self.deployed_contracts.count(),
            "{} extra deployed contract mappings, remaining contract mappings are: {:?}, counted {}.\nready intents are {:#?}\npending intents are {:#?}",
            self.deployed_contracts.count().saturating_sub(deployed_count),
            self.deployed_contracts,
            deployed_count,
            self.tx_intent_queue_ready,
            self.tx_intent_queue_pending_by_nonce
        );
    }
}

impl MempoolInner {
    pub fn new(limits_config: MempoolLimits) -> Self {
        Self {
            nonce_mapping: Default::default(),
            tx_intent_queue_ready: Default::default(),
            tx_intent_queue_pending_by_nonce: Default::default(),
            tx_intent_queue_pending_by_timestamp: Default::default(),
            deployed_contracts: Default::default(),
            limiter: MempoolLimiter::new(limits_config),
            tx_received: Default::default(),
            #[cfg(any(test, feature = "testing"))]
            nonce_cache_inner: Default::default(),
        }
    }

    /// Returns true if at least one transaction can be consumed from the mempool.
    pub fn has_ready_transactions(&self) -> bool {
        !self.tx_intent_queue_ready.is_empty()
    }

    pub fn has_transaction(&self, tx_hash: &TransactionHash) -> bool {
        self.tx_received.contains(tx_hash)
    }

    pub fn n_total(&self) -> usize {
        self.limiter.current_transactions
    }

    /// When `force` is `true`, this function should never return any error.
    /// `update_limits` is `false` when the transaction has been removed from
    /// the mempool in the past without updating the limits.
    pub fn insert_tx(
        &mut self,
        mempool_tx: MempoolTransaction,
        force: bool,
        update_limits: bool,
        nonce_info: NonceInfo,
    ) -> Result<(), TxInsertionError> {
        // delete age-exceeded txs from the mempool
        // todo(perf): this may want to limit this check once every few seconds
        // to avoid it being in the hot path?
        let limits_for_tx = TransactionCheckedLimits::limits_for(&mempool_tx);
        if !force {
            self.remove_age_exceeded_txs();
            self.limiter.check_insert_limits(&limits_for_tx)?;
        }

        let contract_address = mempool_tx.contract_address().to_felt();
        let arrived_at = mempool_tx.arrived_at;
        // DeployAccount
        let tx_hash = mempool_tx.tx_hash();
        let deployed_contract_address = mempool_tx.tx.deployed_contract_address();

        // Inserts the transaction into the nonce tx mapping for the current
        // contract
        match self.nonce_mapping.entry(contract_address) {
            hash_map::Entry::Occupied(mut entry) => {
                // Handle nonce collision.
                let nonce_tx_mapping = entry.get_mut();
                let replaced = match nonce_tx_mapping.insert(mempool_tx, nonce_info.nonce, force) {
                    Ok(replaced) => replaced,
                    Err(nonce_collision_or_duplicate_hash) => {
                        debug_assert!(!force); // Force add should never error
                        return Err(nonce_collision_or_duplicate_hash);
                    }
                };

                // Update the tx queues.
                match nonce_info.readiness {
                    NonceStatus::Ready => {
                        // Remove old value (if collision and force == true)
                        if let ReplacedState::Replaced { previous } = replaced {
                            let removed = self.tx_intent_queue_ready.remove(&TransactionIntentReady {
                                contract_address,
                                timestamp: previous.arrived_at,
                                nonce: nonce_info.nonce,
                                nonce_next: nonce_info.nonce_next,
                                phantom: std::marker::PhantomData,
                            });
                            debug_assert!(removed);
                            self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&previous));

                            // So! This is a pretty nasty edge case. If we
                            // replace a transaction, and the previous tx was
                            // a deploy account, we must DECREMENT the count for
                            // that address. If the NEW transactions is a deploy
                            // but not the old one, we must INCREMENT the count.
                            // Otherwise, we have replaced a deploy account with
                            // another deploy account and we do nothing.
                            if let Some(contract_address) = &deployed_contract_address {
                                if previous.tx.tx_type() != TransactionType::DeployAccount {
                                    self.deployed_contracts.increment(*contract_address);
                                }
                            } else if previous.tx.tx_type() == TransactionType::DeployAccount {
                                self.deployed_contracts.decrement(previous.contract_address());
                            }
                        } else if let Some(contract_address) = &deployed_contract_address {
                            self.deployed_contracts.increment(*contract_address)
                        }

                        // Insert new value
                        let insert = self.tx_intent_queue_ready.insert(TransactionIntentReady {
                            contract_address,
                            timestamp: arrived_at,
                            nonce: nonce_info.nonce,
                            nonce_next: nonce_info.nonce_next,
                            phantom: Default::default(),
                        });
                        debug_assert!(insert);
                    }
                    NonceStatus::Pending => {
                        // The pending queue works a little bit differently as
                        // it is split into individual sub-queues for each
                        // contract address
                        let queue = self.tx_intent_queue_pending_by_nonce.entry(contract_address).or_default();

                        // Remove old value (if collision and force == true)
                        if let ReplacedState::Replaced { previous } = replaced {
                            let removed = queue.remove(&TransactionIntentPendingByNonce {
                                contract_address,
                                timestamp: previous.arrived_at,
                                nonce: nonce_info.nonce,
                                nonce_next: nonce_info.nonce_next,
                                phantom: std::marker::PhantomData,
                            });
                            debug_assert!(removed.is_some());

                            let removed = self.tx_intent_queue_pending_by_timestamp.remove(
                                &TransactionIntentPendingByTimestamp {
                                    contract_address,
                                    timestamp: previous.arrived_at,
                                    nonce: nonce_info.nonce,
                                    nonce_next: nonce_info.nonce_next,
                                    phantom: std::marker::PhantomData,
                                },
                            );
                            debug_assert!(removed);

                            self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&previous));

                            if let Some(contract_address) = &deployed_contract_address {
                                if previous.tx.tx_type() != TransactionType::DeployAccount {
                                    self.deployed_contracts.increment(*contract_address);
                                }
                            } else if previous.tx.tx_type() == TransactionType::DeployAccount {
                                self.deployed_contracts.decrement(previous.contract_address())
                            }
                        } else if let Some(contract_address) = &deployed_contract_address {
                            self.deployed_contracts.increment(*contract_address);
                        }

                        // Insert new value
                        let inserted = queue.insert(
                            TransactionIntentPendingByNonce {
                                contract_address,
                                timestamp: arrived_at,
                                nonce: nonce_info.nonce,
                                nonce_next: nonce_info.nonce_next,
                                phantom: std::marker::PhantomData,
                            },
                            (),
                        );
                        debug_assert!(inserted.is_none());

                        let inserted =
                            self.tx_intent_queue_pending_by_timestamp.insert(TransactionIntentPendingByTimestamp {
                                contract_address,
                                timestamp: arrived_at,
                                nonce: nonce_info.nonce,
                                nonce_next: nonce_info.nonce_next,
                                phantom: std::marker::PhantomData,
                            });
                        debug_assert!(inserted);
                    }
                };
            }
            hash_map::Entry::Vacant(entry) => {
                // Insert the new nonce tx mapping
                let nonce_tx_mapping = NonceTxMapping::new_with_first_tx(mempool_tx, nonce_info.nonce);
                entry.insert(nonce_tx_mapping);

                // Update the tx queues.
                let inserted = match nonce_info.readiness {
                    NonceStatus::Ready => self.tx_intent_queue_ready.insert(TransactionIntentReady {
                        contract_address,
                        timestamp: arrived_at,
                        nonce: nonce_info.nonce,
                        nonce_next: nonce_info.nonce_next,
                        phantom: std::marker::PhantomData,
                    }),
                    NonceStatus::Pending => {
                        let insert_1 =
                            self.tx_intent_queue_pending_by_timestamp.insert(TransactionIntentPendingByTimestamp {
                                contract_address,
                                timestamp: arrived_at,
                                nonce: nonce_info.nonce,
                                nonce_next: nonce_info.nonce_next,
                                phantom: std::marker::PhantomData,
                            });

                        let insert_2 = self
                            .tx_intent_queue_pending_by_nonce
                            .entry(contract_address)
                            .or_default()
                            .insert(
                                TransactionIntentPendingByNonce {
                                    contract_address,
                                    timestamp: arrived_at,
                                    nonce: nonce_info.nonce,
                                    nonce_next: nonce_info.nonce_next,
                                    phantom: std::marker::PhantomData,
                                },
                                (),
                            )
                            .is_none();

                        insert_1 && insert_2
                    }
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

        self.tx_received.insert(tx_hash);
        Ok(())
    }

    pub fn has_deployed_contract(&self, addr: &ContractAddress) -> bool {
        self.deployed_contracts.contains(addr)
    }

    pub fn remove_age_exceeded_txs(&mut self) {
        let mut ready_no_age_check = vec![];

        // We take advantage of the fact that TransactionIntentReady is
        // ordered by timestamp, so as soon as we find a transaction which has
        // not exceeded its max age (and that transaction supports age limits)
        // we know no more transactions can be removed.
        while let Some(intent) = self.tx_intent_queue_ready.first() {
            let hash_map::Entry::Occupied(mut entry) = self.nonce_mapping.entry(intent.contract_address) else {
                unreachable!("Nonce chain does not match tx queue");
            };

            let nonce_mapping = entry.get_mut();
            let btree_map::Entry::Occupied(nonce_mapping_entry) = nonce_mapping.transactions.entry(intent.nonce) else {
                unreachable!("Nonce chain without a tx");
            };

            let limits = TransactionCheckedLimits::limits_for(nonce_mapping_entry.get());
            if self.limiter.tx_age_exceeded(&limits) {
                let mempool_tx = nonce_mapping_entry.remove();
                debug_assert!(
                    self.tx_received.remove(&mempool_tx.tx_hash()),
                    "Tried to remove a ready transaction which had not already been marked as received"
                );

                // We must remember to update the deploy contract count on removal!
                if let Some(contract_address) = mempool_tx.tx.deployed_contract_address() {
                    self.deployed_contracts.decrement(contract_address);
                }

                if nonce_mapping.transactions.is_empty() {
                    entry.remove();
                }

                self.tx_intent_queue_ready.pop_first();
            } else if limits.checks_age() {
                break;
            } else {
                // Some intents are not checked for age. Right now this is only
                // the case for l1 handler intents. If we run into one of those,
                // we still need to check if the intents after it have timed
                // out, so we pop it to get the next. We will then add back the
                // intents which were popped in a separate loop outside this
                // one.
                //
                // In practice this is ok as l1 handler transactions are few and
                // far between. Note that removing this check will result in an
                // infinite loop if ever an l1 transaction is encountered.
                ready_no_age_check.push(self.tx_intent_queue_ready.pop_first().expect("Already inside loop"));
            }
        }

        // Adding back ready transactions with no age check to them
        for intent in ready_no_age_check {
            self.tx_intent_queue_ready.insert(intent);
        }

        let mut pending_no_age_check = vec![];

        // The code for removing age-exceeded pending transactions is similar,
        // but with the added complexity that we need to keep
        // tx_intent_queue_pending and tx_intent_queue_pending_by_timestamp in
        // sync. This means removals need to take place across both queues.
        while let Some(intent) = self.tx_intent_queue_pending_by_timestamp.first() {
            // Set 1: look for a pending transaction which is too old
            let hash_map::Entry::Occupied(mut entry) = self.nonce_mapping.entry(intent.contract_address) else {
                unreachable!("Nonce chain does not match tx queue");
            };

            let nonce_mapping = entry.get_mut();
            let btree_map::Entry::Occupied(nonce_mapping_entry) = nonce_mapping.transactions.entry(intent.nonce) else {
                unreachable!("Nonce chain without a tx");
            };

            let limits = TransactionCheckedLimits::limits_for(nonce_mapping_entry.get());
            if self.limiter.tx_age_exceeded(&limits) {
                // Step 2: we found it! Now we remove the entry in tx_intent_queue_pending_by_timestamp

                let mempool_tx = nonce_mapping_entry.remove(); // *- snip -*
                debug_assert!(
                    self.tx_received.remove(&mempool_tx.tx_hash()),
                    "Tried to remove a pending transaction which had not already been marked as received"
                );

                if let Some(contract_address) = mempool_tx.tx.deployed_contract_address() {
                    // Remember to update the deployed contract count along the way!
                    self.deployed_contracts.decrement(contract_address);
                }

                if nonce_mapping.transactions.is_empty() {
                    entry.remove(); // *- snip -*
                }

                let intent = self
                    .tx_intent_queue_pending_by_timestamp
                    .pop_first() // *- snip -*
                    .expect("Already in loop, first entry must exist");

                // Step 3: we remove the TransactionIntentPendingByNonce
                // associated to the TransactionIntentPendingByTimestamp we just
                // removed.

                let hash_map::Entry::Occupied(mut entry) =
                    self.tx_intent_queue_pending_by_nonce.entry(intent.contract_address)
                else {
                    unreachable!("Missing pending intent mapping for {:?}", intent.contract_address);
                };

                let queue = entry.get_mut();
                let remove = queue.remove(&intent.by_nonce()); // *- snip -*
                debug_assert!(remove.is_some());

                // Step 4: tx_intent_queue_pending is essentially a tree of
                // trees. We cannot leave empty sub-trees or else the memory
                // will fill up! So if the queue of pending intents by nonce is
                // empty, we remove it as well.

                if queue.is_empty() {
                    entry.remove(); // *- snip -*
                }
            } else if limits.checks_age() {
                break;
            } else {
                pending_no_age_check
                    .push(self.tx_intent_queue_pending_by_timestamp.pop_first().expect("Already inside loop"));
            }
        }

        for intent in pending_no_age_check {
            self.tx_intent_queue_pending_by_timestamp.insert(intent);
        }
    }

    pub fn pop_next(&mut self) -> Option<MempoolTransaction> {
        // Pop tx queue.
        let (tx_mempool, contract_address, nonce_next) = loop {
            // Bubble up None if the mempool is empty.
            let tx_intent = self.tx_intent_queue_ready.pop_first()?;
            let tx_mempool = self.pop_tx_from_intent(&tx_intent);

            let limits = TransactionCheckedLimits::limits_for(&tx_mempool);
            if !self.limiter.tx_age_exceeded(&limits) {
                break (tx_mempool, tx_intent.contract_address, tx_intent.nonce_next);
            }

            // transaction age exceeded, remove the tx from mempool.
            self.limiter.mark_removed(&limits);
        };

        // Looks for the next transaction from the same account in the pending
        // queue and marks it as ready if found.
        'pending: {
            if let hash_map::Entry::Occupied(mut entry) = self.tx_intent_queue_pending_by_nonce.entry(contract_address)
            {
                let queue = entry.get_mut();

                let entry_inner = queue.first_entry().expect("Intent queue cannot be empty");
                let nonce = entry_inner.key().nonce;

                if nonce != nonce_next {
                    break 'pending;
                }

                let intent_pending_by_nonce = entry_inner.remove_entry().0;

                // This works like a NonceMapping: if a pending intent queue is
                // empty, we remove the mapping.
                if queue.is_empty() {
                    entry.remove();
                }

                // We need to keep pending intents by timestamp in sync!
                let intent_pending_by_timestamp = intent_pending_by_nonce.by_timestamp();
                let removed = self.tx_intent_queue_pending_by_timestamp.remove(&intent_pending_by_timestamp);
                debug_assert!(removed);

                let intent_ready = intent_pending_by_nonce.ready();
                self.tx_intent_queue_ready.insert(intent_ready);
            }
        }

        #[cfg(any(test, feature = "testing"))]
        self.nonce_cache_inner.insert(tx_mempool.contract_address(), tx_mempool.nonce_next);
        self.limiter.mark_removed(&TransactionCheckedLimits::limits_for(&tx_mempool));
        self.tx_received.remove(&tx_mempool.tx_hash());

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
        if let Some(contract_address) = mempool_tx.tx.deployed_contract_address() {
            self.deployed_contracts.decrement(contract_address);
        }

        mempool_tx
    }

    pub fn pop_next_chunk(&mut self, dest: &mut impl Extend<MempoolTransaction>, n: usize) {
        dest.extend((0..n).map_while(|_| self.pop_next()))
    }

    /// Returns true if [MempoolInner] has the transaction at a contract address
    /// and [Nonce] in the ready queue.
    pub fn nonce_is_ready(&self, sender_address: Felt, nonce: Nonce) -> bool {
        let mempool_tx = self.nonce_mapping.get(&sender_address).map(|mapping| mapping.transactions.get(&nonce));
        let Some(Some(mempool_tx)) = mempool_tx else {
            return false;
        };

        self.tx_intent_queue_ready.contains(&TransactionIntentReady {
            contract_address: sender_address,
            timestamp: mempool_tx.arrived_at,
            nonce,
            nonce_next: mempool_tx.nonce_next,
            phantom: std::marker::PhantomData,
        })
    }

    /// Returns true if [MempoolInner] has a transaction from a contract address
    /// with a specific [Nonce] in _both_ pending queues.
    #[cfg(any(test, feature = "testing"))]
    pub fn nonce_is_pending(&self, sender_address: Felt, nonce: Nonce) -> bool {
        let mempool_tx = self.nonce_mapping.get(&sender_address).map(|mapping| mapping.transactions.get(&nonce));
        let Some(Some(mempool_tx)) = mempool_tx else {
            return false;
        };

        let queue = self
            .tx_intent_queue_pending_by_nonce
            .get(&sender_address)
            .unwrap_or_else(|| panic!("Missing mapping for pending contract address {sender_address:x?}"));

        let contains_by_nonce = || {
            queue.contains_key(&TransactionIntentPendingByNonce {
                contract_address: sender_address,
                timestamp: mempool_tx.arrived_at,
                nonce,
                nonce_next: mempool_tx.nonce_next,
                phantom: std::marker::PhantomData,
            })
        };

        let contains_by_timestamp = || {
            self.tx_intent_queue_pending_by_timestamp.contains(&TransactionIntentPendingByTimestamp {
                contract_address: sender_address,
                timestamp: mempool_tx.arrived_at,
                nonce,
                nonce_next: mempool_tx.nonce_next,
                phantom: std::marker::PhantomData,
            })
        };

        contains_by_nonce() && contains_by_timestamp()
    }

    /// Returns true if [MempoolInner] contains a transaction with a specific
    /// [Nonce] from a given contract address.
    #[cfg(any(test, feature = "testing"))]
    pub fn nonce_exists(&self, sender_address: Felt, nonce: Nonce) -> bool {
        self.nonce_mapping
            .get(&sender_address)
            .map(|mapping| mapping.transactions.contains_key(&nonce))
            .unwrap_or(false)
    }

    /// Returns true if [MempoolInner] contains a transaction with a specific
    /// hash which was emitted by a given contract address.
    #[cfg(any(test, feature = "testing"))]
    pub fn tx_hash_exists(&self, sender_address: Felt, nonce: Nonce, tx_hash: TransactionHash) -> bool {
        let mempool_tx = self.nonce_mapping.get(&sender_address).map(|mapping| mapping.transactions.get(&nonce));

        let Some(Some(mempool_tx)) = mempool_tx else {
            return false;
        };

        mempool_tx.tx_hash() == tx_hash
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_empty(&self) -> bool {
        self.tx_intent_queue_ready.is_empty()
    }
}
