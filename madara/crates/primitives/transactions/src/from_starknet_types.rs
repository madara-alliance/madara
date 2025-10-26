use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl From<mp_rpc::v0_7_1::Txn> for Transaction {
    fn from(tx: mp_rpc::v0_7_1::Txn) -> Self {
        match tx {
            mp_rpc::v0_7_1::Txn::Invoke(tx) => Self::Invoke(tx.into()),
            mp_rpc::v0_7_1::Txn::L1Handler(tx) => Self::L1Handler(tx.into()),
            mp_rpc::v0_7_1::Txn::Declare(tx) => Self::Declare(tx.into()),
            mp_rpc::v0_7_1::Txn::Deploy(tx) => Self::Deploy(tx.into()),
            mp_rpc::v0_7_1::Txn::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}
impl From<mp_rpc::v0_8_1::Txn> for Transaction {
    fn from(tx: mp_rpc::v0_8_1::Txn) -> Self {
        match tx {
            mp_rpc::v0_8_1::Txn::Invoke(tx) => Self::Invoke(tx.into()),
            mp_rpc::v0_8_1::Txn::L1Handler(tx) => Self::L1Handler(tx.into()),
            mp_rpc::v0_8_1::Txn::Declare(tx) => Self::Declare(tx.into()),
            mp_rpc::v0_8_1::Txn::Deploy(tx) => Self::Deploy(tx.into()),
            mp_rpc::v0_8_1::Txn::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}

impl From<mp_rpc::v0_7_1::InvokeTxn> for InvokeTransaction {
    fn from(tx: mp_rpc::v0_7_1::InvokeTxn) -> Self {
        match tx {
            mp_rpc::v0_7_1::InvokeTxn::V0(tx) => Self::V0(tx.into()),
            mp_rpc::v0_7_1::InvokeTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::v0_7_1::InvokeTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}
impl From<mp_rpc::v0_8_1::InvokeTxn> for InvokeTransaction {
    fn from(tx: mp_rpc::v0_8_1::InvokeTxn) -> Self {
        match tx {
            mp_rpc::v0_8_1::InvokeTxn::V0(tx) => Self::V0(tx.into()),
            mp_rpc::v0_8_1::InvokeTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::v0_8_1::InvokeTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::v0_7_1::InvokeTxnV0> for InvokeTransactionV0 {
    fn from(tx: mp_rpc::v0_7_1::InvokeTxnV0) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<mp_rpc::v0_7_1::InvokeTxnV1> for InvokeTransactionV1 {
    fn from(tx: mp_rpc::v0_7_1::InvokeTxnV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<mp_rpc::v0_7_1::InvokeTxnV3> for InvokeTransactionV3 {
    fn from(tx: mp_rpc::v0_7_1::InvokeTxnV3) -> Self {
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
impl From<mp_rpc::v0_8_1::InvokeTxnV3> for InvokeTransactionV3 {
    fn from(tx: mp_rpc::v0_8_1::InvokeTxnV3) -> Self {
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

impl From<mp_rpc::v0_7_1::L1HandlerTxn> for L1HandlerTransaction {
    fn from(tx: mp_rpc::v0_7_1::L1HandlerTxn) -> Self {
        Self {
            version: Felt::from_hex(&tx.version).unwrap_or(Felt::ZERO),
            nonce: tx.nonce,
            contract_address: tx.function_call.contract_address,
            entry_point_selector: tx.function_call.entry_point_selector,
            calldata: tx.function_call.calldata,
        }
    }
}

impl From<mp_rpc::v0_7_1::DeclareTxn> for DeclareTransaction {
    fn from(tx: mp_rpc::v0_7_1::DeclareTxn) -> Self {
        match tx {
            mp_rpc::v0_7_1::DeclareTxn::V0(tx) => Self::V0(tx.into()),
            mp_rpc::v0_7_1::DeclareTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::v0_7_1::DeclareTxn::V2(tx) => Self::V2(tx.into()),
            mp_rpc::v0_7_1::DeclareTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}
impl From<mp_rpc::v0_8_1::DeclareTxn> for DeclareTransaction {
    fn from(tx: mp_rpc::v0_8_1::DeclareTxn) -> Self {
        match tx {
            mp_rpc::v0_8_1::DeclareTxn::V0(tx) => Self::V0(tx.into()),
            mp_rpc::v0_8_1::DeclareTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::v0_8_1::DeclareTxn::V2(tx) => Self::V2(tx.into()),
            mp_rpc::v0_8_1::DeclareTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::v0_7_1::DeclareTxnV0> for DeclareTransactionV0 {
    fn from(tx: mp_rpc::v0_7_1::DeclareTxnV0) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::v0_7_1::DeclareTxnV1> for DeclareTransactionV1 {
    fn from(tx: mp_rpc::v0_7_1::DeclareTxnV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::v0_7_1::DeclareTxnV2> for DeclareTransactionV2 {
    fn from(tx: mp_rpc::v0_7_1::DeclareTxnV2) -> Self {
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

impl From<mp_rpc::v0_7_1::DeclareTxnV3> for DeclareTransactionV3 {
    fn from(tx: mp_rpc::v0_7_1::DeclareTxnV3) -> Self {
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
impl From<mp_rpc::v0_8_1::DeclareTxnV3> for DeclareTransactionV3 {
    fn from(tx: mp_rpc::v0_8_1::DeclareTxnV3) -> Self {
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

impl From<mp_rpc::v0_7_1::DeployTxn> for DeployTransaction {
    fn from(tx: mp_rpc::v0_7_1::DeployTxn) -> Self {
        Self {
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl From<mp_rpc::v0_7_1::DeployAccountTxn> for DeployAccountTransaction {
    fn from(tx: mp_rpc::v0_7_1::DeployAccountTxn) -> Self {
        match tx {
            mp_rpc::v0_7_1::DeployAccountTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::v0_7_1::DeployAccountTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}
impl From<mp_rpc::v0_8_1::DeployAccountTxn> for DeployAccountTransaction {
    fn from(tx: mp_rpc::v0_8_1::DeployAccountTxn) -> Self {
        match tx {
            mp_rpc::v0_8_1::DeployAccountTxn::V1(tx) => Self::V1(tx.into()),
            mp_rpc::v0_8_1::DeployAccountTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::v0_7_1::DeployAccountTxnV1> for DeployAccountTransactionV1 {
    fn from(tx: mp_rpc::v0_7_1::DeployAccountTxnV1) -> Self {
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
impl From<mp_rpc::v0_7_1::DeployAccountTxnV3> for DeployAccountTransactionV3 {
    fn from(tx: mp_rpc::v0_7_1::DeployAccountTxnV3) -> Self {
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
impl From<mp_rpc::v0_8_1::DeployAccountTxnV3> for DeployAccountTransactionV3 {
    fn from(tx: mp_rpc::v0_8_1::DeployAccountTxnV3) -> Self {
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
