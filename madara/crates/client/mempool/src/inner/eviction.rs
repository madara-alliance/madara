use crate::inner::{
    accounts::AccountUpdate,
    tx::{AccountKey, EvictionScore},
};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct EvictionQueue(BTreeSet<(EvictionScore, AccountKey)>);

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
impl EvictionQueue {
    pub fn check_invariants(&self, accounts: &crate::accounts::Accounts) {
        // Matches backing datastructure
        let expected = accounts
            .all_accounts()
            .iter()
            .map(|(contract_address, account)| {
                (account.eviction_score().expect("No eviction score for account"), AccountKey(*contract_address))
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(self.0, expected);
    }
}

impl EvictionQueue {
    pub fn apply_account_update(&mut self, account_update: &AccountUpdate) {
        if account_update.account_data.new_eviction_score != account_update.account_data.previous_eviction_score {
            // changed

            if let Some(previous) = &account_update.account_data.previous_eviction_score {
                let removed = self.0.remove(&(previous.clone(), account_update.account_key));
                assert!(removed)
            }

            if let Some(new) = &account_update.account_data.new_eviction_score {
                let inserted = self.0.insert((new.clone(), account_update.account_key));
                assert!(inserted)
            }
        }
    }

    /// Get the next account where we can evict a transaction if it is less desirable than `other`.
    pub fn get_next_if_less_desirable_than(&self, other: &EvictionScore) -> Option<&AccountKey> {
        // We want the highest score which is last.
        self.0.last().filter(|e| e.0 > *other).map(|e| &e.1)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
