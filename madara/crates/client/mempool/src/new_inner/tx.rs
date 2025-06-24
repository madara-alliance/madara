use crate::new_inner::TxInsertionError;
use mp_transactions::validated::{TxTimestamp, ValidatedMempoolTx};
use starknet_api::{
    core::{ContractAddress, Nonce},
    transaction::TransactionHash,
};

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq, Clone))]
pub struct MempoolTransaction {
    inner: ValidatedMempoolTx,
    score: Score,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvictionScore {
    /// Last tx nonce of the account - current account nonce
    pub chain_nonce_len: Nonce,
    pub score: Score,
}

impl EvictionScore {
    pub fn new(tx: &MempoolTransaction, account_nonce: Nonce) -> Self {
        Self { chain_nonce_len: Nonce(tx.nonce().0 - account_nonce.0), score: tx.score }
    }
}

impl MempoolTransaction {
    pub fn new(inner: ValidatedMempoolTx, score_function: &ScoreFunction) -> MempoolTransaction {
        Self { score: score_function.get_score(&inner), inner }
    }
    pub fn into_inner(self) -> ValidatedMempoolTx {
        self.inner
    }
    pub fn info(&self) -> TxInfo {
        TxInfo {
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
        Nonce(self.inner.tx.nonce())
    }
    pub fn contract_address(&self) -> ContractAddress {
        todo!()
    }
    pub fn tx_hash(&self) -> TransactionHash {
        todo!()
    }
    pub fn arrived_at(&self) -> TxTimestamp {
        todo!()
    }
    pub fn account_key(&self) -> AccountKey {
        AccountKey(self.contract_address())
    }
    pub fn tx_key(&self) -> TxKey {
        TxKey(self.contract_address(), self.nonce())
    }
    pub fn is_declare(&self) -> bool {
        self.inner.tx.as_deploy().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreFunction {
    /// FCFS mode. Transaction that have arrived earlier will be prioritised.
    Timestamp,
    /// Tip mode. Transactions with higher tip will be prioritised.
    Tip {
        /// Min tip bump to replace a transaction.
        min_tip_bump: u128,
    },
}

/// Opaque score, defined by the score function. When comparing transactions, the higher score will always have priority.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct Score(u128);

impl ScoreFunction {
    pub fn get_score(&self, tx: &ValidatedMempoolTx) -> Score {
        match self {
            // Reverse the order, so that higher score means priority.
            Self::Timestamp => Score(u128::MAX - tx.arrived_at.0),
            Self::Tip { .. } => todo!(),
        }
    }

    pub fn check_tip_bump(
        &self,
        previous_tx: &MempoolTransaction,
        new_tx: &MempoolTransaction,
    ) -> Result<(), TxInsertionError> {
        match self {
            // FCFS serve never supports replacing a transaction.
            Self::Timestamp => Err(TxInsertionError::NonceConflict),
            Self::Tip { min_tip_bump } => {
                if new_tx.score().0.saturating_sub(previous_tx.score().0) < *min_tip_bump {
                    return Err(TxInsertionError::DuplicateTxn);
                }
                Ok(())
            }
        }
    }
}

/// Key to index a transaction in the mempool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TxKey(pub ContractAddress, pub Nonce);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountKey(pub ContractAddress);

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(PartialEq, Eq))]
pub struct TxInfo {
    pub nonce: Nonce,
    pub contract_address: ContractAddress,
    pub score: Score,
    pub arrived_at: TxTimestamp,
    pub tx_hash: TransactionHash,
    pub is_declare: bool,
}

impl TxInfo {
    pub fn account_key(&self) -> AccountKey {
        AccountKey(self.contract_address)
    }
    pub fn tx_key(&self) -> TxKey {
        TxKey(self.contract_address, self.nonce)
    }
}
