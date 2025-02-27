use crate::Transaction;
use crate::TransactionWithHash;

use blockifier::transaction::transaction_execution::Transaction as BTransaction;
impl From<BTransaction> for TransactionWithHash {
    fn from(tx: BTransaction) -> TransactionWithHash {
        match tx {
            BTransaction::Account(tx) => tx.tx.into(),
            BTransaction::L1Handler(tx) => tx.into(),
        }
    }
}

use starknet_api::executable_transaction::Transaction as ApiTransaction;
impl From<ApiTransaction> for TransactionWithHash {
    fn from(tx: ApiTransaction) -> TransactionWithHash {
        match tx {
            ApiTransaction::Account(tx) => tx.into(),
            ApiTransaction::L1Handler(tx) => tx.into(),
        }
    }
}

use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
impl From<ApiAccountTransaction> for TransactionWithHash {
    fn from(tx: ApiAccountTransaction) -> TransactionWithHash {
        match tx {
            ApiAccountTransaction::Declare(tx) => tx.into(),
            ApiAccountTransaction::DeployAccount(tx) => tx.into(),
            ApiAccountTransaction::Invoke(tx) => tx.into(),
        }
    }
}

use starknet_api::executable_transaction::DeclareTransaction as ApiDeclareTransaction;
impl From<ApiDeclareTransaction> for TransactionWithHash {
    fn from(tx: ApiDeclareTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::Declare(tx.tx.into()), hash: *tx.tx_hash }
    }
}

use starknet_api::executable_transaction::DeployAccountTransaction as ApiDeployAccountTransaction;
impl From<ApiDeployAccountTransaction> for TransactionWithHash {
    fn from(tx: ApiDeployAccountTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::DeployAccount(tx.tx.into()), hash: *tx.tx_hash }
    }
}

use starknet_api::executable_transaction::InvokeTransaction as ApiInvokeTransaction;
impl From<ApiInvokeTransaction> for TransactionWithHash {
    fn from(tx: ApiInvokeTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::Invoke(tx.tx.into()), hash: *tx.tx_hash }
    }
}

use starknet_api::executable_transaction::L1HandlerTransaction as ApiL1HandlerTransaction;
impl From<ApiL1HandlerTransaction> for TransactionWithHash {
    fn from(tx: ApiL1HandlerTransaction) -> TransactionWithHash {
        TransactionWithHash { transaction: Transaction::L1Handler(tx.tx.into()), hash: *tx.tx_hash }
    }
}
