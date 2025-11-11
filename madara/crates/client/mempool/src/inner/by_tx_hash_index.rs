use crate::{inner::accounts::AccountUpdate, tx::TxKey};
use starknet_api::transaction::TransactionHash;
use std::collections::HashMap;

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct ByTxHashIndex(HashMap<TransactionHash, TxKey>);

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
impl ByTxHashIndex {
    pub fn check_invariants(&self, accounts: &crate::accounts::Accounts) {
        // Matches backing datastructure
        let expected = accounts
            .all_accounts()
            .iter()
            .flat_map(|(contract_address, account)| {
                account.queued_txs.iter().map(|(nonce, tx)| (tx.tx_hash(), tx.tx_key()))
            })
            .collect::<HashMap<_, _>>();

        assert_eq!(self.0, expected);
    }
}

impl ByTxHashIndex {
    pub fn apply_account_update(&mut self, account_update: &AccountUpdate) {
        for removed_tx in &account_update.removed_txs {
            let res = self.0.remove(&removed_tx.tx_hash());
            assert!(res.is_some(), "Invariant violated: Transaction should be in the by hash index.");
        }
        if let Some(added_tx) = &account_update.added_tx {
            let res = self.0.insert(added_tx.tx_hash, added_tx.tx_key());
            assert!(res.is_none(), "Invariant violated: Transaction should not already be in the hash index.");
        }
    }

    pub fn contains(&self, hash: &TransactionHash) -> bool {
        self.0.contains_key(hash)
    }

    pub fn get(&self, hash: &TransactionHash) -> Option<&TxKey> {
        self.0.get(hash)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
