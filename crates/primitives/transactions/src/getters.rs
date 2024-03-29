use alloc::vec::Vec;

use mp_felt::Felt252Wrapper;
use starknet_api::transaction::{DeclareTransactionV0V1, DeclareTransactionV2, InvokeTransactionV0, InvokeTransactionV1, Transaction};

use super::{DeclareTransaction, DeployAccountTransaction, InvokeTransaction, UserTransaction};
use crate::{
    HandleL1MessageTransaction, UserOrL1HandlerTransaction,
};

impl UserTransaction {
    pub fn sender_address(&self) -> Felt252Wrapper {
        match self {
            UserTransaction::Declare(tx, _) => (*tx.sender_address()).into(),
            UserTransaction::DeployAccount(tx) => tx.account_address(),
            UserTransaction::Invoke(tx) => *tx.sender_address(),
        }
    }

    pub fn signature(&self) -> &Vec<Felt252Wrapper> {
        match self {
            UserTransaction::Declare(tx, _) => tx.signature(),
            UserTransaction::DeployAccount(tx) => tx.signature(),
            UserTransaction::Invoke(tx) => tx.signature(),
        }
    }

    pub fn max_fee(&self) -> &u128 {
        match self {
            UserTransaction::Declare(tx, _) => tx.max_fee(),
            UserTransaction::DeployAccount(tx) => tx.max_fee(),
            UserTransaction::Invoke(tx) => tx.max_fee(),
        }
    }

    pub fn calldata(&self) -> Option<&Vec<Felt252Wrapper>> {
        match self {
            UserTransaction::Declare(..) => None,
            UserTransaction::DeployAccount(tx) => Some(tx.calldata()),
            UserTransaction::Invoke(tx) => Some(tx.calldata()),
        }
    }

    pub fn nonce(&self) -> Option<&Felt252Wrapper> {
        match self {
            UserTransaction::Declare(tx, _) => Some(tx.nonce()),
            UserTransaction::DeployAccount(tx) => Some(tx.nonce()),
            UserTransaction::Invoke(tx) => tx.nonce(),
        }
    }

    pub fn offset_version(&self) -> bool {
        match self {
            UserTransaction::Declare(tx, _) => tx.offset_version(),
            UserTransaction::DeployAccount(tx) => tx.offset_version(),
            UserTransaction::Invoke(tx) => tx.offset_version(),
        }
    }
}

pub trait TransactionVersion {
    fn version(&self) -> u8;
}

impl TransactionVersion for UserTransaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        match self {
            UserTransaction::Declare(tx, _) => tx.version(),
            UserTransaction::DeployAccount(tx) => tx.version(),
            UserTransaction::Invoke(tx) => tx.version(),
        }
    }
}

impl TransactionVersion for Transaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        match self {
            Transaction::Declare(tx) => tx.version(),
            Transaction::Deploy(tx) => tx.version(),
            Transaction::DeployAccount(tx) => tx.version(),
            Transaction::Invoke(tx) => tx.version(),
            Transaction::L1Handler(tx) => tx.version(),
        }
    }
}

impl TransactionVersion for UserOrL1HandlerTransaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        match self {
            UserOrL1HandlerTransaction::User(tx) => tx.version(),
            UserOrL1HandlerTransaction::L1Handler(tx, _) => tx.version(),
        }
    }
}

impl TransactionVersion for InvokeTransaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        match self {
            InvokeTransaction::V0(tx) => tx.version(),
            InvokeTransaction::V1(tx) => tx.version(),
            InvokeTransaction::V3(tx) => tx.version(),
        }
    }
}

impl TransactionVersion for InvokeTransactionV0 {
    #[inline(always)]
    fn version(&self) -> u8 {
        0
    }
}

impl TransactionVersion for InvokeTransactionV1 {
    #[inline(always)]
    fn version(&self) -> u8 {
        1
    }
}

impl TransactionVersion for DeclareTransaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        match self {
            DeclareTransaction::V0(tx) => tx.version(),
            DeclareTransaction::V1(tx) => tx.version(),
            DeclareTransaction::V2(tx) => tx.version(),
            DeclareTransaction::V3(tx) => tx.version(),
        }
    }
}

impl TransactionVersion for DeclareTransactionV0V1 {
    #[inline(always)]
    fn version(&self) -> u8 {
        0
    }
}

// TODO: what should we do here?
// impl TransactionVersion for DeclareTransactionV1 {
//     #[inline(always)]
//     fn version(&self) -> u8 {
//         1
//     }
// }

impl TransactionVersion for DeclareTransactionV2 {
    #[inline(always)]
    fn version(&self) -> u8 {
        2
    }
}

impl TransactionVersion for DeployAccountTransaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        1
    }
}

impl TransactionVersion for HandleL1MessageTransaction {
    #[inline(always)]
    fn version(&self) -> u8 {
        0
    }
}
