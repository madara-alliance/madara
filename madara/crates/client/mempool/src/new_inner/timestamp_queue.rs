use crate::new_inner::{accounts::AccountUpdate, tx::TxKey};
use mp_transactions::validated::TxTimestamp;
use std::collections::BTreeSet;

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct TimestampQueue(BTreeSet<(TxTimestamp, TxKey)>);

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
impl TimestampQueue {
    pub fn check_invariants(&self, accounts: &crate::accounts::Accounts) {
        // Matches backing datastructure
        let expected = accounts
            .all_accounts()
            .iter()
            .flat_map(|(contract_address, account)| {
                account.queued_txs.iter().map(|(nonce, tx)| (tx.arrived_at(), tx.tx_key()))
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(self.0, expected);
    }
}

impl TimestampQueue {
    pub fn apply_account_update(&mut self, account_update: &AccountUpdate) {
        for removed_tx in &account_update.removed_txs {
            let res = self.0.remove(&(removed_tx.arrived_at(), removed_tx.tx_key()));
            assert!(res, "Invariant violated: Transaction should be in the timestamp queue.");
        }
        if let Some(added_tx) = &account_update.added_tx {
            let res = self.0.insert((added_tx.arrived_at, added_tx.tx_key()));
            assert!(res, "Invariant violated: Transaction should be added in the timestamp queue.");
        }
    }

    pub fn first_older_than(&self, ts: TxTimestamp) -> Option<&TxKey> {
        // Oldest is first (min `arrived_at`)
        self.0.first().filter(|tx| tx.0 < ts).map(|e| &e.1)
    }
}
