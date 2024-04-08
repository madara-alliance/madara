use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use mp_felt::Felt252Wrapper;
use starknet_api::transaction::TransactionHash;

use crate::TxType;

pub trait Getters {
    fn sender_address(&self) -> Felt252Wrapper;
    fn signature(&self) -> Vec<Felt252Wrapper>;
    fn calldata(&self) -> Option<Vec<Felt252Wrapper>>;
    fn nonce(&self) -> Option<Felt252Wrapper>;
    fn tx_type(&self) -> TxType;
}

pub trait Hash {
    fn tx_hash(&self) -> Option<TransactionHash>;
}

impl Getters for AccountTransaction {
    fn sender_address(&self) -> Felt252Wrapper {
        match self {
            AccountTransaction::Declare(tx) => tx.tx.sender_address().into(),
            AccountTransaction::DeployAccount(tx) => tx.tx.contract_address_salt().into(),
            AccountTransaction::Invoke(tx) => tx.tx.sender_address().into(),
        }
    }

    fn signature(&self) -> Vec<Felt252Wrapper> {
        match self {
            AccountTransaction::Declare(tx) => {
                tx.tx.signature().0.iter().map(|x| Felt252Wrapper::from(*x).into()).collect()
            }
            AccountTransaction::DeployAccount(tx) => {
                tx.tx.signature().0.iter().map(|x| Felt252Wrapper::from(*x).into()).collect()
            }
            AccountTransaction::Invoke(tx) => {
                tx.tx.signature().0.iter().map(|x| Felt252Wrapper::from(*x).into()).collect()
            }
        }
    }

    fn calldata(&self) -> Option<Vec<Felt252Wrapper>> {
        match self {
            AccountTransaction::Declare(..) => None,
            AccountTransaction::DeployAccount(tx) => {
                Some(tx.tx.constructor_calldata().0.iter().map(|x| Felt252Wrapper::from(*x).into()).collect())
            }
            AccountTransaction::Invoke(tx) => {
                Some(tx.tx.calldata().0.iter().map(|x| Felt252Wrapper::from(*x).into()).collect())
            }
        }
    }

    fn nonce(&self) -> Option<Felt252Wrapper> {
        match self {
            AccountTransaction::Declare(tx) => Some(tx.tx.nonce().0.into()),
            AccountTransaction::DeployAccount(tx) => Some(tx.tx.nonce().0.into()),
            AccountTransaction::Invoke(tx) => Some(tx.tx.nonce().0.into()),
        }
    }

    fn tx_type(&self) -> TxType {
        match self {
            AccountTransaction::Declare(..) => TxType::Declare,
            AccountTransaction::DeployAccount(..) => TxType::DeployAccount,
            AccountTransaction::Invoke(..) => TxType::Invoke,
        }
    }
}

impl Hash for Transaction {
    fn tx_hash(&self) -> Option<TransactionHash> {
        match self {
            Transaction::AccountTransaction(tx) => tx.tx_hash(),
            Transaction::L1HandlerTransaction(tx) => Some(tx.tx_hash),
        }
    }
}

impl Hash for AccountTransaction {
    fn tx_hash(&self) -> Option<TransactionHash> {
        match self {
            AccountTransaction::Declare(tx) => Some(tx.tx_hash),
            AccountTransaction::DeployAccount(tx) => Some(tx.tx_hash),
            AccountTransaction::Invoke(tx) => Some(tx.tx_hash),
        }
    }
}

// impl UserOrL1HandlerTransaction {
//     pub fn tx_type(&self) -> TxType {
//         match self {
//             UserOrL1HandlerTransaction::User(user_tx) => match user_tx {
//                 AccountTransaction::Declare(_) => TxType::Declare,
//                 AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
//                 AccountTransaction::Invoke(_) => TxType::Invoke,
//             },
//             UserOrL1HandlerTransaction::L1Handler(_) => TxType::L1Handler,
//         }
//     }

//     pub fn tx_hash(&self) -> Option<TransactionHash> {
//         match self {
//             UserOrL1HandlerTransaction::User(user_tx) => match user_tx {
//                 AccountTransaction::Declare(declare_transaction) =>
// Some(declare_transaction.tx_hash),
// AccountTransaction::DeployAccount(deploy_account_transaction) => {
// Some(deploy_account_transaction.tx_hash)                 }
//                 AccountTransaction::Invoke(invoke_transaction) =>
// Some(invoke_transaction.tx_hash),             },
//             UserOrL1HandlerTransaction::L1Handler(l1_handler_transaction) =>
// Some(l1_handler_transaction.tx_hash),         }
//     }
// }

// pub trait TransactionVersion {
//     fn version(&self) -> u8;
// }

// impl TransactionVersion for InvokeTransaction {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         match self {
//             InvokeTransaction::V0(tx) => tx.version(),
//             InvokeTransaction::V1(tx) => tx.version(),
//             InvokeTransaction::V3(tx) => tx.version(),
//         }
//     }
// }

// impl TransactionVersion for InvokeTransactionV0 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         0
//     }
// }

// impl TransactionVersion for InvokeTransactionV1 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         1
//     }
// }

// impl TransactionVersion for InvokeTransactionV3 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         3
//     }
// }

// impl TransactionVersion for DeclareTransaction {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         match self {
//             DeclareTransaction::V0(tx) => tx.version(),
//             DeclareTransaction::V1(tx) => tx.version(),
//             DeclareTransaction::V2(tx) => tx.version(),
//             DeclareTransaction::V3(tx) => tx.version(),
//         }
//     }
// }

// impl TransactionVersion for DeclareTransactionV0V1 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         0
//     }
// }

// // TODO: what should we do here?
// // impl TransactionVersion for DeclareTransactionV1 {
// //     #[inline(always)]
// //     fn version(&self) -> u8 {
// //         1
// //     }
// // }

// impl TransactionVersion for DeclareTransactionV2 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         2
//     }
// }

// impl TransactionVersion for DeclareTransactionV3 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         3
//     }
// }

// impl TransactionVersion for DeployAccountTransaction {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         1
//     }
// }

// impl TransactionVersion for L1HandlerTransaction {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         0
//     }
// }
