use crate::Transaction;
use crate::TransactionWithHash;

use blockifier::transaction::transaction_execution::Transaction as BTransaction;
impl From<BTransaction> for TransactionWithHash {
    fn from(tx: BTransaction) -> TransactionWithHash {
        match tx {
            BTransaction::AccountTransaction(tx) => tx.into(),
            BTransaction::L1HandlerTransaction(tx) => tx.into(),
        }
    }
}

use blockifier::transaction::account_transaction::AccountTransaction as BAccountTransaction;
impl From<BAccountTransaction> for TransactionWithHash {
    fn from(tx: BAccountTransaction) -> TransactionWithHash {
        match tx {
            BAccountTransaction::Declare(tx) => tx.into(),
            BAccountTransaction::DeployAccount(tx) => tx.into(),
            BAccountTransaction::Invoke(tx) => tx.into(),
        }
    }
}

use blockifier::transaction::transactions::DeclareTransaction as BDeclareTransaction;
impl From<BDeclareTransaction> for TransactionWithHash {
    fn from(tx: BDeclareTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::Declare(tx.tx.into()), hash: *tx.tx_hash }
    }
}

use blockifier::transaction::transactions::DeployAccountTransaction as BDeployAccountTransaction;
impl From<BDeployAccountTransaction> for TransactionWithHash {
    fn from(tx: BDeployAccountTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::DeployAccount(tx.tx.into()), hash: *tx.tx_hash }
    }
}

use blockifier::transaction::transactions::InvokeTransaction as BInvokeTransaction;
impl From<BInvokeTransaction> for TransactionWithHash {
    fn from(tx: BInvokeTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::Invoke(tx.tx.into()), hash: *tx.tx_hash }
    }
}

use blockifier::transaction::transactions::L1HandlerTransaction as BL1HandlerTransaction;
impl From<BL1HandlerTransaction> for TransactionWithHash {
    fn from(tx: BL1HandlerTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::L1Handler(tx.tx.into()), hash: *tx.tx_hash }
    }
}
