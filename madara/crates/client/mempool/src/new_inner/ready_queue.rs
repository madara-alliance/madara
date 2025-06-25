use crate::{
    accounts::AccountStatus,
    new_inner::{accounts::AccountUpdate, tx::AccountKey},
    tx::Score,
};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct ReadyQueue(BTreeSet<(Score, AccountKey)>);

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
impl ReadyQueue {
    pub fn check_invariants(&self, accounts: &crate::accounts::Accounts) {
        // Matches backing datastructure
        let expected = accounts
            .all_accounts()
            .iter()
            .filter_map(|(contract_address, account)| {
                if let AccountStatus::Ready(score) = account.status() {
                    Some((score, AccountKey(*contract_address)))
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(self.0, expected);
    }
}

impl ReadyQueue {
    pub fn apply_account_update(&mut self, account_update: &AccountUpdate) {
        if account_update.account_data.previous_status != account_update.account_data.new_status {
            // Status changed
            if let AccountStatus::Ready(score) = account_update.account_data.previous_status {
                let res = self.0.remove(&(score, account_update.account_key));
                assert!(res, "Invariant violated: Transaction should be in the ready queue.");
            }
            if let AccountStatus::Ready(score) = account_update.account_data.new_status {
                let res = self.0.insert((score, account_update.account_key));
                assert!(res, "Invariant violated: Transaction should be added to the ready queue.");
            }
        }
    }

    pub fn get_first(&self) -> Option<&AccountKey> {
        self.0.first().map(|acc| &acc.1)
    }

    pub fn has_ready_transactions(&self) -> bool {
        !self.0.is_empty()
    }

    pub fn ready_transactions(&self) -> usize {
        self.0.len()
    }
}
