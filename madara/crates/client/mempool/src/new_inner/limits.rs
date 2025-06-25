use crate::new_inner::{accounts::AccountUpdate, tx::MempoolTransaction};

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
struct MempoolLimiterConfig {
    max_transactions: usize,
    max_declare_transactions: Option<usize>,
}

#[derive(Debug, Default)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
struct MempoolLimiterState {
    transactions: usize,
    declare_transactions: usize,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum MempoolLimitReached {
    #[error("The mempool has reached the limit of {max} transactions")]
    MaxTransactions { max: usize },
    #[error("The mempool has reached the limit of {max} declare transactions")]
    MaxDeclareTransactions { max: usize },
}

impl MempoolLimitReached {
    pub fn can_trigger_eviction_policy(&self) -> bool {
        matches!(self, Self::MaxTransactions { .. })
    }
}

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub(crate) struct MempoolLimiter {
    config: MempoolLimiterConfig,
    state: MempoolLimiterState,
}

#[cfg(any(test, feature = "testing"))]
#[allow(unused)]
impl MempoolLimiter {
    pub fn check_invariants(&self, accounts: &crate::accounts::Accounts) {
        // Matches backing datastructure
        let count: usize = accounts.all_accounts().values().map(|acc| acc.queued_txs.len()).sum();
        assert_eq!(self.state.transactions, count, "Invalid transaction count state");
        let declared: usize = accounts
            .all_accounts()
            .values()
            .map(|acc| acc.queued_txs.values().filter(|e| e.is_declare()).count())
            .sum();
        assert_eq!(self.state.declare_transactions, declared, "Invalid declared count state");

        // Is less than limit
        assert!(
            self.state.transactions < self.config.max_transactions,
            "Mempool has {} < {} tx limit",
            self.state.transactions,
            self.config.max_transactions
        );
        if let Some(declare_max) = self.config.max_declare_transactions {
            assert!(
                self.state.declare_transactions < declare_max,
                "Mempool has {} < {} declare tx limit",
                self.state.declare_transactions,
                declare_max
            );
        }
    }
}

impl MempoolLimiter {
    pub fn new(config: &super::InnerMempoolConfig) -> Self {
        Self {
            config: MempoolLimiterConfig {
                max_transactions: config.max_transactions,
                max_declare_transactions: config.max_declare_transactions,
            },
            state: Default::default(),
        }
    }

    pub fn check_room_for_replacement(
        &self,
        previous_tx: &MempoolTransaction,
        new_tx: &MempoolTransaction,
    ) -> Result<(), MempoolLimitReached> {
        if let Some(max) = self.config.max_declare_transactions {
            // Adding a new declare tx
            if new_tx.is_declare() && !previous_tx.is_declare() && self.state.declare_transactions >= max {
                return Err(MempoolLimitReached::MaxDeclareTransactions { max });
            }
        }

        Ok(())
    }

    pub fn check_room_for_new_tx(&self, tx: &MempoolTransaction) -> Result<(), MempoolLimitReached> {
        if let Some(max) = self.config.max_declare_transactions {
            // Adding a new declare tx
            if tx.is_declare() && self.state.declare_transactions >= max {
                return Err(MempoolLimitReached::MaxDeclareTransactions { max });
            }
        }

        if self.state.transactions >= self.config.max_transactions {
            return Err(MempoolLimitReached::MaxTransactions { max: self.config.max_transactions });
        }

        Ok(())
    }

    pub fn apply_account_update(&mut self, account_update: &AccountUpdate) {
        for tx in &account_update.removed_txs {
            if tx.is_declare() {
                self.state.declare_transactions -= 1;
            }
            self.state.transactions -= 1
        }

        if let Some(tx) = &account_update.added_tx {
            if tx.is_declare {
                self.state.declare_transactions += 1;
            }
            self.state.transactions += 1
        }
    }
}
