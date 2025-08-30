use crate::inner::TxInsertionError;
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
};

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct MempoolTransaction {
    pub inner: ValidatedTransaction,
    pub score: Score,
}

/// Eviction score. Lower is better.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionScore {
    /// Last tx nonce of the account - current account nonce
    pub chain_nonce_len: Nonce,
    pub score: Score,
}

impl Ord for EvictionScore {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // For two accounts with differing chain nonce len, higher eviction score means the one with the biggest chain_nonce_len
        // For two accounts with same chain nonce len, the one with the lowest score has higher eviction score (we want to get rid of low score txs first in this case)
        // See tests::test_eviction_score_order in this same file.
        self.chain_nonce_len.cmp(&other.chain_nonce_len).then(self.score.cmp(&other.score).reverse())
    }
}
impl PartialOrd for EvictionScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl EvictionScore {
    pub fn new(tx: &MempoolTransaction, account_nonce: Nonce) -> Self {
        Self { chain_nonce_len: Nonce(tx.nonce().0 - account_nonce.0), score: tx.score }
    }
}

impl MempoolTransaction {
    pub fn new(
        inner: ValidatedTransaction,
        score_function: &ScoreFunction,
    ) -> Result<MempoolTransaction, TxInsertionError> {
        let _: ContractAddress =
            inner.contract_address.try_into().map_err(|_| TxInsertionError::InvalidContractAddress)?;
        Ok(Self { score: score_function.get_score(&inner), inner })
    }
    pub fn into_inner(self) -> ValidatedTransaction {
        self.inner
    }
    pub fn info(&self) -> TxSummary {
        TxSummary {
            nonce: self.nonce(),
            contract_address: self.contract_address(),
            score: self.score(),
            arrived_at: self.arrived_at(),
            tx_hash: self.tx_hash(),
            is_declare: self.is_declare(),
        }
    }
    pub fn score(&self) -> Score {
        self.score
    }
    pub fn nonce(&self) -> Nonce {
        Nonce(self.inner.transaction.nonce())
    }
    pub fn contract_address(&self) -> ContractAddress {
        self.inner.contract_address.try_into().expect("Invalid contract address")
    }
    pub fn tx_hash(&self) -> TransactionHash {
        TransactionHash(self.inner.hash)
    }
    pub fn arrived_at(&self) -> TxTimestamp {
        self.inner.arrived_at
    }
    pub fn account_key(&self) -> AccountKey {
        AccountKey(self.contract_address())
    }
    pub fn tx_key(&self) -> TxKey {
        TxKey(self.contract_address(), self.nonce())
    }
    pub fn is_declare(&self) -> bool {
        self.inner.transaction.as_declare().is_some()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScoreFunction {
    /// FCFS mode. Transaction that have arrived earlier will be prioritised.
    Timestamp,
    /// Tip mode. Transactions with higher tip will be prioritised.
    Tip {
        /// Min tip bump to replace a transaction, as ratio.
        min_tip_bump: f64,
    },
}

/// Opaque score, defined by the score function. When comparing transactions, the higher score will always have priority.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy, Default)]
pub struct Score(pub u64);

impl ScoreFunction {
    pub fn get_score(&self, tx: &ValidatedTransaction) -> Score {
        match self {
            // Reverse the order, so that higher score means priority.
            Self::Timestamp => Score(u64::MAX - tx.arrived_at.0),
            Self::Tip { .. } => Score(tx.transaction.tip().unwrap_or(0)),
        }
    }

    pub fn check_tip_bump(
        &self,
        previous_tx: &MempoolTransaction,
        new_tx: &MempoolTransaction,
    ) -> Result<(), TxInsertionError> {
        match self {
            Self::Timestamp => {
                // FCFS will always replace newer txs with older txs. This is important when re-adding transactions if a
                // pre-confirmed block is not confirmed, for example. The transactions will be re-added to the mempool.
                if new_tx.arrived_at() >= previous_tx.arrived_at() {
                    return Err(TxInsertionError::NonceConflict);
                }
            }
            Self::Tip { min_tip_bump } => {
                // `as` keyword: conversion to f64 is lossy but infaillible. We intentionally avoid the opposite conversion here.
                let min_tip = previous_tx.score().0 as f64 * (1.0 + *min_tip_bump);
                if (new_tx.score.0 as f64) < min_tip {
                    return Err(TxInsertionError::MinTipBump { min_tip_bump: *min_tip_bump });
                }
            }
        }
        Ok(())
    }
}

/// Key to index a transaction in the mempool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TxKey(pub ContractAddress, pub Nonce);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountKey(pub ContractAddress);

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq))]
pub struct TxSummary {
    pub nonce: Nonce,
    pub contract_address: ContractAddress,
    pub score: Score,
    pub arrived_at: TxTimestamp,
    pub tx_hash: TransactionHash,
    pub is_declare: bool,
}

impl TxSummary {
    pub fn account_key(&self) -> AccountKey {
        AccountKey(self.contract_address)
    }
    pub fn tx_key(&self) -> TxKey {
        TxKey(self.contract_address, self.nonce)
    }
}

#[cfg(test)]
mod tests {
    use starknet_api::core::Nonce;
    use std::cmp::Ordering;

    use crate::tx::{EvictionScore, Score};

    #[test]
    fn test_eviction_score_order() {
        assert_eq!(
            std::cmp::Ord::cmp(
                &EvictionScore { chain_nonce_len: Nonce((1).into()), score: Score(5) },
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(5) }
            ),
            Ordering::Less
        );
        assert_eq!(
            std::cmp::Ord::cmp(
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(10) },
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(5) }
            ),
            Ordering::Less
        );
        assert_eq!(
            std::cmp::Ord::cmp(
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(5) },
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(5) }
            ),
            Ordering::Equal
        );
        assert_eq!(
            std::cmp::Ord::cmp(
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(5) },
                &EvictionScore { chain_nonce_len: Nonce((1).into()), score: Score(5) }
            ),
            Ordering::Greater
        );
        assert_eq!(
            std::cmp::Ord::cmp(
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(5) },
                &EvictionScore { chain_nonce_len: Nonce((5).into()), score: Score(10) }
            ),
            Ordering::Greater
        );
    }
}
