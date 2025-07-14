use crate::inner::{
    accounts::{AccountUpdate, Accounts},
    by_tx_hash_index::ByTxHashIndex,
    eviction::EvictionQueue,
    limits::{MempoolLimitReached, MempoolLimiter},
    ready_queue::ReadyQueue,
    timestamp_queue::TimestampQueue,
    tx::{EvictionScore, MempoolTransaction, ScoreFunction},
};
use mp_transactions::validated::{TxTimestamp, ValidatedMempoolTx};
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
};
use std::time::Duration;

pub(crate) mod accounts;
pub(crate) mod by_tx_hash_index;
pub(crate) mod eviction;
pub(crate) mod limits;
pub(crate) mod ready_queue;
pub(crate) mod tests;
pub(crate) mod timestamp_queue;
pub(crate) mod tx;

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum TxInsertionError {
    #[error("Nonce mismatch: account nonce is {account_nonce:?}")]
    NonceTooLow { account_nonce: Nonce },
    #[error("A transaction with this nonce already exists in the mempool")]
    NonceConflict,
    #[error("A transaction with this hash already exists in the mempool")]
    DuplicateTxn,
    #[error("Replacing a transaction requires at least a tip bump of at least {min_tip_bump} units")]
    MinTipBump { min_tip_bump: u128 },
    #[error("Transaction is too old; max age is {ttl:?}")]
    TooOld { ttl: Duration },
    #[error("Cannot add a declare transaction with a future nonce")]
    PendingDeclare,
    #[error("Invalid contract address")]
    InvalidContractAddress,
    /// Transactions without tips are not supported when using the mempool in tip mode.
    #[error("Cannot add a transaction without a tip into the mempool")]
    NoTip,
    #[error(transparent)]
    Limit(#[from] MempoolLimitReached),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerMempoolConfig {
    pub score_function: ScoreFunction,
    pub max_transactions: usize,
    pub max_declare_transactions: Option<usize>,
    pub ttl: Option<Duration>,
}

// Implementation details:
//
// The inner mempool is a complex datastructure which can be indexed in a number of ways, of which:
// - Insertion requires indexing by a (account_address, nonce) tuple, and requires keeping track of an account nonce for every account.
// - A ready queue, sorted by score (tip/timestamp), is needed for picking transactions for block building.
// - A timestamp queue is needed for removing transactions that have exceeded their TTL (time-to-live).
// - A mapping from transaction hash to transaction is required for getting the status of a transaction by hash.
// - When inserting a transaction in a full mempool, we need to evict less desirable transactions to make space.
// - etc.
//
// This results in a lot of invariants to consider, as a single piece of data (status of an account or transaction) is deduplicated
// in a ton of places, and every mutation needs to update all of the places to be correct. To remain sane, and to make the mempool
// easily extensible, the datastructure is written so that:
// - The main backing datastructure is the `Accounts` struct, which you can think of a
//  `HashMap<ContractAddress, AccountData { current_nonce: Nonce, queued_txs: SortedMap<Nonce, Tx> }>`. This holds every account in the mempool, as
//  well as all of the queued tx for all accounts.
// - Therefore, in this struct, every transaction can be uniquely queried using a `TxKey(ContractAddress, Nonce)`, and every account using an `AccountKey(ContractAddress)`.
// - The InnerMempool contains secondary datastructure for indexing: the ready queue, timestamp queue, etc. These need to reflect any change made to the backing `Accounts`
//  datastructure.
//
// With this design, any mutation to the InnerMempool can follow these specific steps:
// 1) Without modifying the InnerMempool, figure out the update you want to make and perform every pre-check needed.
// 2) Update the backing `Accounts` datastructure with your change. Once it is updated, you are not allowed to early-return or error from this point on.
// 3) Update all of the secondary datastructures to reflect the primary change. This is made totally generic of the specifics of the change thanks to the
//   `AccountUpdate` struct.
// By keeping this `AccountUpdate` generic, every invariant is local to its datastructure and this simplifies their handling.

/// The in-memory datastructure containing all of the mempool transactions. Mutations to this datastructure are synchronous, via &mut references to ensure
/// atomicity.
#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct InnerMempool {
    config: InnerMempoolConfig,

    /// Backing datastructure, single source of truth for the other datastructure. Contains account nonces and queued txs for each account.
    accounts: Accounts,

    // Secondary indices
    /// Limiter: apply mempool limits.
    limiter: MempoolLimiter,
    /// Queue of ready accounts sorted by front tx score.
    ready_queue: ReadyQueue,
    /// Queue of all transactions sorted by arrived_at timestamp.
    timestamp_queue: TimestampQueue,
    /// Index TxHash => TxKey.
    by_tx_hash: ByTxHashIndex,
    /// Queue of all acounts sorted by eviction score of the last queued tx of each account.
    eviction_queue: EvictionQueue,
}

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
impl InnerMempool {
    pub fn check_invariants(&self) {
        self.accounts.check_invariants();
        self.limiter.check_invariants(&self.accounts);
        self.ready_queue.check_invariants(&self.accounts);
        self.timestamp_queue.check_invariants(&self.accounts);
        self.by_tx_hash.check_invariants(&self.accounts);
        self.eviction_queue.check_invariants(&self.accounts);
    }

    pub fn get_account_nonce(&self, contract_address: &ContractAddress) -> Option<&Nonce> {
        self.accounts.all_accounts().get(contract_address).map(|acc| &acc.current_nonce)
    }

    pub fn account_nonces(&self) -> impl Iterator<Item = (&ContractAddress, &Nonce)> {
        self.accounts
            .all_accounts()
            .iter()
            .map(|(contract_address, account)| (contract_address, &account.current_nonce))
    }

    pub fn transactions(&self) -> impl Iterator<Item = &ValidatedMempoolTx> {
        self.accounts.all_accounts().iter().flat_map(|(_, acc)| acc.queued_txs.values()).map(|t| &t.inner)
    }
}

impl InnerMempool {
    pub fn new(config: InnerMempoolConfig) -> Self {
        Self {
            limiter: MempoolLimiter::new(&config),
            config,
            accounts: Default::default(),
            ready_queue: Default::default(),
            timestamp_queue: Default::default(),
            by_tx_hash: Default::default(),
            eviction_queue: Default::default(),
        }
    }

    /// Update indices based on an account update, and extend `removed_txs` with the removed txs.
    fn apply_update(&mut self, account_update: AccountUpdate, removed_txs: &mut impl Extend<ValidatedMempoolTx>) {
        tracing::debug!("Apply update {account_update:?}");
        self.ready_queue.apply_account_update(&account_update);
        self.timestamp_queue.apply_account_update(&account_update);
        self.by_tx_hash.apply_account_update(&account_update);
        self.eviction_queue.apply_account_update(&account_update);
        self.limiter.apply_account_update(&account_update);
        removed_txs.extend(account_update.removed_txs.into_iter().map(|el| el.into_inner()));
    }

    /// Insert a transaction into the mempool.
    ///
    /// ## Arguments
    ///
    /// * `now`: current time.
    /// * `account_nonce`: the current nonce for the contract_address. Caller is responsible for getting it from the backend.
    ///   If the account already exists within the mempool, this argument will be ignored.
    /// * `removed_txs`: if any transaction is removed from the mempool during insertion. This helps the caller do bookkeeping
    ///   if necessary (remove from db, send update notifications...)
    ///
    /// ## Returns
    ///
    /// Returns nothing, or an error if the transaction was rejected. If an error is returned, the transaction is not inserted.
    pub fn insert_tx(
        &mut self,
        now: TxTimestamp,
        tx: ValidatedMempoolTx,
        account_nonce: Nonce,
        removed_txs: &mut impl Extend<ValidatedMempoolTx>,
    ) -> Result<(), TxInsertionError> {
        // Prechecks: TTL
        if let Some(ttl) = self.config.ttl {
            if tx.arrived_at <= now.checked_sub(ttl).unwrap_or(TxTimestamp::UNIX_EPOCH) {
                return Err(TxInsertionError::TooOld { ttl });
            }
        }
        let mempool_tx = MempoolTransaction::new(tx, &self.config.score_function)?;

        // Entry in the backing accounts datastructure. This will not insert the tx into the mempool yet.
        let mut entry = self.accounts.tx_entry_for_insertion(&mempool_tx, account_nonce)?;
        let account_nonce = entry.updated_data().account_nonce;

        // Prechecks before insertion

        // Declare-specific checks: declare transaction must be directly ready.
        // TODO: in v0.14 declare transactions have a specific queue where they're delayed. For now we don't need more than this.
        if mempool_tx.is_declare() && account_nonce != mempool_tx.nonce() {
            return Err(TxInsertionError::PendingDeclare);
        }

        // If we're replacing another transactions,
        if let Some(previous_tx) = entry.replaced_tx() {
            // If it's the same tx, show a nicer error message.
            if previous_tx.tx_hash() == mempool_tx.tx_hash() {
                return Err(TxInsertionError::DuplicateTxn);
            }

            // Limiter check
            self.limiter.check_room_for_replacement(previous_tx, &mempool_tx)?;
            // Check tip bump
            self.config.score_function.check_tip_bump(previous_tx, &mempool_tx)?;
        }
        // Otherwise, we are adding a new transaction into a new slot.
        else {
            // Limiter check
            if let Err(err) = self.limiter.check_room_for_new_tx(&mempool_tx) {
                if !err.can_trigger_eviction_policy() {
                    return Err(err.into());
                }
                // Try to make space by evicting less desirable transactions.
                let new_tx_eviction_score = EvictionScore::new(&mempool_tx, account_nonce);
                tracing::debug!("Try make room: {new_tx_eviction_score:?}");
                if !self.try_make_room_for(&new_tx_eviction_score, removed_txs) {
                    return Err(err.into()); // Failed to make room
                }
                // We made room!

                // Reborrow the insertion `entry`, as `try_make_room_for` had to borrow it mutably to make its modifications.
                entry = self
                    .accounts
                    .tx_entry_for_insertion(&mempool_tx, account_nonce)
                    .expect("Insertion should not fail after making room");
                assert!(entry.replaced_tx().is_none(), "Entry should be the same");
            }
        }

        // We're clear to insert into the entry :)
        let account_update = entry.insert(mempool_tx);
        self.apply_update(account_update, removed_txs);

        Ok(())
    }

    /// Applies the [EvictionScore] policy: we remove the least desirable transaction in the mempool if it is less desirable than this
    /// new one.
    fn try_make_room_for(&mut self, new_tx: &EvictionScore, removed_txs: &mut impl Extend<ValidatedMempoolTx>) -> bool {
        let Some(account_key) = self.eviction_queue.get_next_if_less_desirable_than(new_tx) else {
            return false;
        };

        let account_update = self.accounts.pop_last_tx_from_account(account_key);
        self.apply_update(account_update, removed_txs);

        true // we made room! :)
    }

    /// Update an account nonce. This gets rid of all obselete transactions, and needs to be called everytime
    /// new state (statediff NonceUpdate) should be applied.
    ///
    /// ## Arguments
    ///
    /// * `contract_address`: the contract address
    /// * `account_nonce`: the new account nonce
    /// * `removed_txs`: if any transaction is removed from the mempool during insertion.
    ///   This helps the caller do bookkeeping if necessary (remove from db, send update notifications...)
    pub fn update_account_nonce(
        &mut self,
        contract_address: &ContractAddress,
        account_nonce: &Nonce,
        removed_txs: &mut impl Extend<ValidatedMempoolTx>,
    ) {
        let Some(account_update) = self.accounts.update_account_nonce(contract_address, account_nonce) else { return };
        self.apply_update(account_update, removed_txs);
    }

    /// Pop the next ready transaction for block building, or `None` if the mempool has no ready transaction.
    /// This does not increment the nonce of the account, meaning the next transactions for the accounts will not be ready until an
    /// `update_account_nonce` is issued.
    pub fn pop_next_ready(&mut self) -> Option<ValidatedMempoolTx> {
        // Get the next ready account,
        let account_key = self.ready_queue.get_next_ready()?;

        // Remove from the backing datastructure.
        let mut account_update = self.accounts.remove_ready_tx(account_key);

        // Update indices.
        self.limiter.apply_account_update(&account_update);
        self.ready_queue.apply_account_update(&account_update);
        self.timestamp_queue.apply_account_update(&account_update);
        self.by_tx_hash.apply_account_update(&account_update);
        self.eviction_queue.apply_account_update(&account_update);

        assert_eq!(
            account_update.removed_txs.len(),
            1,
            "remove_ready_tx should remove exactly one tx from the mempool"
        );
        account_update.removed_txs.pop().map(|tx| tx.into_inner())
    }

    /// Remove all TTL-exceeded transactions. This needs to be called periodically.
    ///
    /// ## Arguments
    ///
    /// * `now`: current time.
    /// * `removed_txs`: if any transaction is removed from the mempool during insertion. This helps
    ///   the caller do bookkeeping if necessary (remove from db, send update notifications...)
    pub fn remove_all_ttl_exceeded_txs(&mut self, now: TxTimestamp, removed_txs: &mut impl Extend<ValidatedMempoolTx>) {
        let Some(ttl) = self.config.ttl else { return };
        let limit_ts = now.checked_sub(ttl).unwrap_or(TxTimestamp::UNIX_EPOCH);
        while let Some(tx_key) = self.timestamp_queue.first_older_than(limit_ts) {
            let account_update = self.accounts.remove_tx(tx_key);
            self.apply_update(account_update, removed_txs);
        }
    }

    pub fn get_transaction_by_hash(&self, tx_hash: &TransactionHash) -> Option<&ValidatedMempoolTx> {
        self.by_tx_hash.get(tx_hash).and_then(|key| self.accounts.get_tx_by_key(key)).map(|tx| &tx.inner)
    }

    pub fn get_transaction(&self, contract_address: &ContractAddress, nonce: &Nonce) -> Option<&ValidatedMempoolTx> {
        self.accounts.get_transaction(contract_address, nonce).map(|tx| &tx.inner)
    }

    pub fn contains_tx_by_hash(&self, tx_hash: &TransactionHash) -> bool {
        self.by_tx_hash.contains(tx_hash)
    }

    pub fn has_ready_transactions(&self) -> bool {
        self.ready_queue.has_ready_transactions()
    }

    pub fn ready_transactions(&self) -> usize {
        self.ready_queue.ready_transactions()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}
