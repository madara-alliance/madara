use crate::{
    new_inner::{
        tx::{AccountKey, EvictionScore, MempoolTransaction, TxKey},
        TxInsertionError,
    },
    tx::{Score, TxInfo},
};
use mp_convert::Felt;
use starknet_api::core::{ContractAddress, Nonce};
use std::{
    collections::{btree_map, hash_map, BTreeMap, HashMap},
    iter,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    /// Is ready to be executed (account nonce == first tx nonce)
    Ready(/* score of the first tx */ Score),
    /// Has a front nonce gap.
    Pending,
    /// Account has no longer any transaction and is removed from the mempool.
    Removed,
}

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq))]
pub struct AccountUpdateData {
    /// Status of the account after the update.
    pub account_nonce: Nonce,
    pub previous_status: AccountStatus,
    pub new_status: AccountStatus,
    /// The eviction score of an account is based on the last queued transaction. This value is per-account,
    /// as when an account is chosen for eviction, its last queued transaction is removed and the eviction
    /// score is updated.
    pub previous_eviction_score: Option<EvictionScore>,
    pub new_eviction_score: Option<EvictionScore>,
}

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq))]
pub struct AccountUpdate {
    pub account_key: AccountKey,

    /// Removed transactions.
    // SmallVec: this will only use the heap when there is more than 1 mempool tx here.
    pub removed_txs: smallvec::SmallVec<[MempoolTransaction; 1]>,
    /// Added transaction.
    pub added_tx: Option<TxInfo>,

    /// Updated account data.
    pub account_data: AccountUpdateData,
}

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct AccountState {
    pub current_nonce: Nonce,
    pub queued_txs: BTreeMap<Nonce, MempoolTransaction>,
}

impl AccountState {
    pub fn new_with_first_tx(account_nonce: Nonce, tx: MempoolTransaction) -> Self {
        Self { current_nonce: account_nonce, queued_txs: [(tx.nonce(), tx)].into() }
    }

    /// Pop the transactions that are now invalid due to a `current_nonce` update, and return them.
    pub fn pop_invalid_txs(&mut self) -> impl Iterator<Item = MempoolTransaction> + '_ {
        // TODO: use extract_if once it's stable (much more efficient)
        iter::from_fn(|| {
            let entry = self.queued_txs.first_entry()?;
            if entry.get().nonce() < self.current_nonce {
                Some(entry.remove())
            } else {
                None
            }
        })
    }

    pub fn remove_tx(&mut self, nonce: Nonce) -> Option<MempoolTransaction> {
        self.queued_txs.remove(&nonce)
    }

    pub fn pop_first(&mut self) -> Option<MempoolTransaction> {
        self.queued_txs.pop_first().map(|kv| kv.1)
    }
    pub fn pop_last(&mut self) -> Option<MempoolTransaction> {
        self.queued_txs.pop_last().map(|kv| kv.1)
    }

    /// Get the status, based on the front queued tx and current nonce.
    pub fn status(&self) -> AccountStatus {
        let Some(tx) = self.queued_txs.first_key_value().map(|kv| kv.1) else { return AccountStatus::Removed };

        if tx.nonce() == self.current_nonce {
            AccountStatus::Ready(tx.score())
        } else {
            AccountStatus::Pending
        }
    }

    pub fn last_queued_tx(&self) -> Option<&'_ MempoolTransaction> {
        self.queued_txs.last_key_value().map(|kv| kv.1)
    }

    /// The eviction score of an account is based on the last queued transaction. This value is per-account,
    /// as when an account is chosen for eviction, its last queued transaction is removed and the eviction
    /// score is updated.
    pub fn eviction_score(&self) -> Option<EvictionScore> {
        self.last_queued_tx().map(|tx| EvictionScore::new(tx, self.current_nonce))
    }
}

enum TxEntryForInsertionInner<'a> {
    /// New mempool account.
    NewAccount { account_nonce: Nonce, entry: hash_map::VacantEntry<'a, ContractAddress, AccountState> },
    /// Replace a transaction in an existing account, at a given nonce.
    Replace(btree_map::OccupiedEntry<'a, Nonce, MempoolTransaction>),
    /// Add a transaction in an existing account, at a given nonce.
    Add(btree_map::VacantEntry<'a, Nonce, MempoolTransaction>),
}

pub struct TxEntryForInsertion<'a> {
    tx_key: TxKey,
    inner: TxEntryForInsertionInner<'a>,
    update_data: AccountUpdateData,
}

impl<'a> TxEntryForInsertion<'a> {
    /// Returns the replaced transaction if inserting this new entry will replace a transaction.
    pub fn replaced_tx(&self) -> Option<&MempoolTransaction> {
        match &self.inner {
            TxEntryForInsertionInner::Replace(entry) => Some(entry.get()),
            _ => None,
        }
    }

    /// New account status once this entry is inserted.
    pub fn updated_data(&self) -> &AccountUpdateData {
        &self.update_data
    }

    pub fn insert(self, tx: MempoolTransaction) -> AccountUpdate {
        let info = tx.info();
        assert_eq!(self.tx_key, tx.tx_key());

        use TxEntryForInsertionInner::*;
        match self.inner {
            NewAccount { account_nonce, entry } => {
                entry.insert(AccountState::new_with_first_tx(account_nonce, tx));
            }
            Replace(mut entry) => {
                entry.insert(tx);
            }
            Add(entry) => {
                entry.insert(tx);
            }
        }

        AccountUpdate {
            account_key: info.account_key(),
            removed_txs: Default::default(),
            added_tx: Some(info),
            account_data: self.update_data,
        }
    }
}

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct Accounts {
    accounts: HashMap<ContractAddress, AccountState>,
}

#[cfg(any(any(test, feature = "testing"), feature = "testing"))]
#[allow(unused)]
impl Accounts {
    pub fn check_invariants(&self) {
        // Invariant: every account should have at least 1 queued tx.
        for (contract_address, account) in &self.accounts {
            assert!(!account.queued_txs.is_empty(), "Account {contract_address:?} has no queued tx");
        }

        // Invariant: queued tx key matches tx
        for (contract_address, account) in &self.accounts {
            for (nonce, tx) in &account.queued_txs {
                assert_eq!(*nonce, tx.nonce(), "Account {contract_address:?} tx has bad nonce key");
            }
        }

        // Invariant: every account must have queued txs that have nonce >= account nonce.
        for (contract_address, account) in &self.accounts {
            let bad_nonces =
                account.queued_txs.iter().filter(|(nonce, _)| **nonce < account.current_nonce).collect::<Vec<_>>();
            assert_eq!(bad_nonces, vec![], "Account {contract_address:?} has queued txs < account nonce");
        }
    }
    pub fn all_accounts(&self) -> &HashMap<ContractAddress, AccountState> {
        &self.accounts
    }
}

impl Accounts {
    /// This does not perform any modification until you actually call the `.insert(tx)` method on the returned entry.
    /// `account_nonce` is only used when the account does not exist in the mempool yet. It will be used to initialize the current nonce.
    pub fn tx_entry_for_insertion(
        &mut self,
        tx: &MempoolTransaction,
        account_nonce: Nonce,
    ) -> Result<TxEntryForInsertion, TxInsertionError> {
        let (inner, update_data) = match self.accounts.entry(tx.contract_address()) {
            // Existing mempool account.
            hash_map::Entry::Occupied(entry) => {
                let account_nonce = entry.get().current_nonce;
                if tx.nonce() < account_nonce {
                    return Err(TxInsertionError::NonceTooLow { account_nonce });
                }

                let previous_status = entry.get().status();
                // Get the new status of the account if the tx were to be inserted.
                let new_status = if tx.nonce() == account_nonce {
                    // This new tx is at the front.
                    AccountStatus::Ready(tx.score())
                } else {
                    // This new tx is not at the front: status is unchanged.
                    previous_status
                };

                let last_queued_tx =
                    entry.get().last_queued_tx().expect("Invariant violation: account has no queued tx");
                let previous_eviction_score = EvictionScore::new(last_queued_tx, account_nonce);
                // Get the eviction score of the account if the tx were to be inserted.
                let new_eviction_score = if tx.nonce() >= last_queued_tx.nonce() {
                    // We are inserting at the end of the account tx queue. We will be the new last queued tx.
                    EvictionScore::new(tx, account_nonce)
                } else {
                    previous_eviction_score.clone() // Unchanged eviction data.
                };

                let entry = match entry.into_mut().queued_txs.entry(tx.nonce()) {
                    // Replace an existing tx at that nonce.
                    btree_map::Entry::Occupied(entry) => TxEntryForInsertionInner::Replace(entry),
                    // Add a new tx for this nonce.
                    btree_map::Entry::Vacant(entry) => TxEntryForInsertionInner::Add(entry),
                };

                (
                    entry,
                    AccountUpdateData {
                        account_nonce,
                        previous_status,
                        new_status,
                        previous_eviction_score: Some(previous_eviction_score),
                        new_eviction_score: Some(new_eviction_score),
                    },
                )
            }
            // No existing mempool account.
            hash_map::Entry::Vacant(entry) => {
                let new_status =
                    if tx.nonce() == account_nonce { AccountStatus::Ready(tx.score()) } else { AccountStatus::Pending };
                (
                    TxEntryForInsertionInner::NewAccount { account_nonce, entry },
                    AccountUpdateData {
                        account_nonce,
                        previous_status: AccountStatus::Removed,
                        new_status,
                        previous_eviction_score: None, // No previous eviction data
                        new_eviction_score: Some(EvictionScore::new(tx, account_nonce)),
                    },
                )
            }
        };

        Ok(TxEntryForInsertion { tx_key: tx.tx_key(), inner, update_data })
    }

    // Helper to fill in the AccountUpdateData for most of the mutations. (everything uses it except insert tx)
    // Important: Caller needs to return `Some` when an update needs to be made, `None` otherwise.
    fn account_update_helper<R>(
        &mut self,
        contract_address: ContractAddress,
        update_fn: impl FnOnce(&mut AccountState) -> Option<R>,
    ) -> Option<(AccountUpdateData, R)> {
        let hash_map::Entry::Occupied(mut account_entry) = self.accounts.entry(contract_address) else { return None };

        let previous_eviction_score = account_entry.get().eviction_score();
        let previous_status = account_entry.get().status();

        let ret = update_fn(account_entry.get_mut())?;

        let new_eviction_score = account_entry.get().eviction_score();
        let account_nonce = account_entry.get().current_nonce;
        let new_status = account_entry.get().status();
        if new_status == AccountStatus::Removed {
            account_entry.remove(); // No more tx for this account.
        }

        Some((
            AccountUpdateData {
                account_nonce,
                previous_status,
                new_status,
                previous_eviction_score,
                new_eviction_score,
            },
            ret,
        ))
    }

    /// This function may return multiple removed txs.
    pub fn update_account_nonce(
        &mut self,
        contract_address: ContractAddress,
        account_nonce: Nonce,
    ) -> Option<AccountUpdate> {
        let (account_data, removed_txs) = self.account_update_helper(contract_address, move |account_state| {
            account_state.current_nonce = account_nonce;
            let removed_txs = account_state.pop_invalid_txs().collect();

            Some(removed_txs)
        })?;

        Some(AccountUpdate { account_key: AccountKey(contract_address), removed_txs, added_tx: None, account_data })
    }

    /// Caller must supply a valid AccountKey for an account in ready state.
    pub fn pop_ready_tx_increment_account_nonce(&mut self, AccountKey(contract_address): AccountKey) -> AccountUpdate {
        let Some((account_data, removed)) = self.account_update_helper(contract_address, move |account_state| {
            let removed = account_state.pop_first().expect("Invariant violation: account entry is empty");
            assert_eq!(removed.nonce(), account_state.current_nonce, "Tried to pop a tx queue that was not ready");
            account_state.current_nonce.0 += Felt::ONE; // increment

            Some(removed)
        }) else {
            // not found
            unreachable!("Invariant violation: AccountKey is invalid: contract_address={contract_address:?}")
        };

        AccountUpdate {
            account_key: AccountKey(contract_address),
            removed_txs: [removed].into(),
            added_tx: None,
            account_data,
        }
    }

    /// Caller must supply a valid TxKey.
    pub fn remove_tx(&mut self, TxKey(contract_address, nonce): TxKey) -> AccountUpdate {
        let Some((data, removed)) = self.account_update_helper(contract_address, move |account_state| {
            let Some(removed) = account_state.remove_tx(nonce) else {
                unreachable!(
                    "Invariant violation: TxKey has invalid nonce: contract_address={contract_address:?} nonce={nonce:?}."
                )
            };

            Some(removed)
        }) else {
            // not found
            unreachable!("Invariant violation: TxKey has invalid contract_address: contract_address={contract_address:?} nonce={nonce:?}.")
        };

        AccountUpdate {
            account_key: AccountKey(contract_address),
            removed_txs: [removed].into(),
            added_tx: None,
            account_data: data,
        }
    }

    /// Caller must supply a valid AccountKey.
    /// This function is used with the eviction queue to make room when the mempool is full.
    pub fn pop_last_tx_from_account(&mut self, AccountKey(contract_address): AccountKey) -> AccountUpdate {
        let Some((account_data, removed)) = self.account_update_helper(contract_address, move |account_state| {
            let removed = account_state.pop_last().expect("Invariant violation: account entry is empty");

            Some(removed)
        }) else {
            // not found
            unreachable!("Invariant violation: AccountKey is invalid: contract_address={contract_address:?}")
        };

        AccountUpdate {
            account_key: AccountKey(contract_address),
            removed_txs: [removed].into(),
            added_tx: None,
            account_data,
        }
    }
}
