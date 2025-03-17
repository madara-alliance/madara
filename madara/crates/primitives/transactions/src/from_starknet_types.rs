use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl From<mp_rpc::Txn> for Transaction {
    fn from(tx: mp_rpc::Txn) -> Self {
        match tx {
            mp_rpc::Txn::Invoke(tx) => Self::Invoke(tx.into()),
            mp_rpc::Txn::L1Handler(tx) => Self::L1Handler(tx.into()),
            mp_rpc::Txn::Declare(tx) => Self::Declare(tx.into()),
            mp_rpc::Txn::Deploy(tx) => Self::Deploy(tx.into()),
            mp_rpc::Txn::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}

impl From<mp_rpc::InvokeTxn> for InvokeTransaction {
    fn from(tx: mp_rpc::InvokeTxn) -> Self {
        match tx {
            mp_rpc::InvokeTxn::V0(tx) => Self::V0(tx.into()),
            mp_rpc::InvokeTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::InvokeTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::InvokeTxnV0> for InvokeTransactionV0 {
    fn from(tx: mp_rpc::InvokeTxnV0) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<mp_rpc::InvokeTxnV1> for InvokeTransactionV1 {
    fn from(tx: mp_rpc::InvokeTxnV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<mp_rpc::InvokeTxnV3> for InvokeTransactionV3 {
    fn from(tx: mp_rpc::InvokeTxnV3) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: tx.nonce,
            resource_bounds: tx.resource_bounds.into(),
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}

impl From<mp_rpc::L1HandlerTxn> for L1HandlerTransaction {
    fn from(tx: mp_rpc::L1HandlerTxn) -> Self {
        Self {
            version: Felt::from_hex(&tx.version).unwrap_or(Felt::ZERO),
            nonce: tx.nonce,
            contract_address: tx.function_call.contract_address,
            entry_point_selector: tx.function_call.entry_point_selector,
            calldata: tx.function_call.calldata,
        }
    }
}

impl From<mp_rpc::DeclareTxn> for DeclareTransaction {
    fn from(tx: mp_rpc::DeclareTxn) -> Self {
        match tx {
            mp_rpc::DeclareTxn::V0(tx) => Self::V0(tx.into()),
            mp_rpc::DeclareTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::DeclareTxn::V2(tx) => Self::V2(tx.into()),
            mp_rpc::DeclareTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::DeclareTxnV0> for DeclareTransactionV0 {
    fn from(tx: mp_rpc::DeclareTxnV0) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::DeclareTxnV1> for DeclareTransactionV1 {
    fn from(tx: mp_rpc::DeclareTxnV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::DeclareTxnV2> for DeclareTransactionV2 {
    fn from(tx: mp_rpc::DeclareTxnV2) -> Self {
        Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::DeclareTxnV3> for DeclareTransactionV3 {
    fn from(tx: mp_rpc::DeclareTxnV3) -> Self {
        Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
            resource_bounds: tx.resource_bounds.into(),
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}

impl From<mp_rpc::DeployTxn> for DeployTransaction {
    fn from(tx: mp_rpc::DeployTxn) -> Self {
        Self {
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::DeployAccountTxn> for DeployAccountTransaction {
    fn from(tx: mp_rpc::DeployAccountTxn) -> Self {
        match tx {
            mp_rpc::DeployAccountTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::DeployAccountTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::DeployAccountTxnV1> for DeployAccountTransactionV1 {
    fn from(tx: mp_rpc::DeployAccountTxnV1) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}
impl From<mp_rpc::DeployAccountTxnV3> for DeployAccountTransactionV3 {
    fn from(tx: mp_rpc::DeployAccountTxnV3) -> Self {
        Self {
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
            resource_bounds: tx.resource_bounds.into(),
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}
