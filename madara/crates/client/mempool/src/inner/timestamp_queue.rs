use crate::inner::{accounts::AccountUpdate, tx::TxKey};
use mp_transactions::validated::TxTimestamp;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct TimestampQueue {
    by_timestamp: BTreeSet<(TxTimestamp, TxKey)>,
    by_insertion: BTreeMap<u64, TxKey>,
    insertion_ordinals: BTreeMap<TxKey, u64>,
    next_insertion_ordinal: u64,
}

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

        assert_eq!(self.by_timestamp, expected);
        assert_eq!(self.by_insertion.len(), expected.len());
        assert_eq!(self.insertion_ordinals.len(), expected.len());
        for (ordinal, tx_key) in &self.by_insertion {
            assert_eq!(self.insertion_ordinals.get(tx_key), Some(ordinal));
            assert!(expected.iter().any(|(_, expected_key)| expected_key == tx_key));
        }
    }
}

impl TimestampQueue {
    pub fn first_inserted(&self) -> Option<&TxKey> {
        self.by_insertion.first_key_value().map(|(_, tx_key)| tx_key)
    }

    pub fn apply_account_update(&mut self, account_update: &AccountUpdate) {
        for removed_tx in &account_update.removed_txs {
            let tx_key = removed_tx.tx_key();
            let res = self.by_timestamp.remove(&(removed_tx.arrived_at(), tx_key));
            assert!(res, "Invariant violated: Transaction should be in the timestamp queue.");
            let ordinal = self
                .insertion_ordinals
                .remove(&tx_key)
                .expect("Invariant violated: Transaction should have an insertion ordinal.");
            assert_eq!(self.by_insertion.remove(&ordinal), Some(tx_key));
        }
        if let Some(added_tx) = &account_update.added_tx {
            let tx_key = added_tx.tx_key();
            let res = self.by_timestamp.insert((added_tx.arrived_at, tx_key));
            assert!(res, "Invariant violated: Transaction should be added in the timestamp queue.");
            let ordinal = self.next_insertion_ordinal;
            self.next_insertion_ordinal =
                self.next_insertion_ordinal.checked_add(1).expect("Insertion ordinal overflow");
            assert_eq!(self.insertion_ordinals.insert(tx_key, ordinal), None);
            assert_eq!(self.by_insertion.insert(ordinal, tx_key), None);
        }
    }

    pub fn first_older_than(&self, ts: TxTimestamp) -> Option<&TxKey> {
        // Oldest is first (min `arrived_at`)
        self.by_timestamp.first().filter(|tx| tx.0 < ts).map(|e| &e.1)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TxKey> {
        self.by_timestamp.iter().map(|(_, tx_key)| tx_key)
    }
}
