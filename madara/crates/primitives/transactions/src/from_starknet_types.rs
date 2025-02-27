use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl From<starknet_types_rpc::Txn<Felt>> for Transaction {
    fn from(tx: starknet_types_rpc::Txn<Felt>) -> Self {
        match tx {
            starknet_types_rpc::Txn::Invoke(tx) => Self::Invoke(tx.into()),
            starknet_types_rpc::Txn::L1Handler(tx) => Self::L1Handler(tx.into()),
            starknet_types_rpc::Txn::Declare(tx) => Self::Declare(tx.into()),
            starknet_types_rpc::Txn::Deploy(tx) => Self::Deploy(tx.into()),
            starknet_types_rpc::Txn::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}

impl From<starknet_types_rpc::InvokeTxn<Felt>> for InvokeTransaction {
    fn from(tx: starknet_types_rpc::InvokeTxn<Felt>) -> Self {
        match tx {
            starknet_types_rpc::InvokeTxn::V0(tx) => Self::V0(tx.into()),
            starknet_types_rpc::InvokeTxn::V1(tx) => Self::V1(tx.into()),
            starknet_types_rpc::InvokeTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_types_rpc::InvokeTxnV0<Felt>> for InvokeTransactionV0 {
    fn from(tx: starknet_types_rpc::InvokeTxnV0<Felt>) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<starknet_types_rpc::InvokeTxnV1<Felt>> for InvokeTransactionV1 {
    fn from(tx: starknet_types_rpc::InvokeTxnV1<Felt>) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<starknet_types_rpc::InvokeTxnV3<Felt>> for InvokeTransactionV3 {
    fn from(tx: starknet_types_rpc::InvokeTxnV3<Felt>) -> Self {
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

impl From<starknet_types_rpc::L1HandlerTxn<Felt>> for L1HandlerTransaction {
    fn from(tx: starknet_types_rpc::L1HandlerTxn<Felt>) -> Self {
        Self {
            version: Felt::from_hex(&tx.version).unwrap_or(Felt::ZERO),
            nonce: tx.nonce,
            contract_address: tx.function_call.contract_address,
            entry_point_selector: tx.function_call.entry_point_selector,
            calldata: tx.function_call.calldata,
        }
    }
}

impl From<starknet_types_rpc::DeclareTxn<Felt>> for DeclareTransaction {
    fn from(tx: starknet_types_rpc::DeclareTxn<Felt>) -> Self {
        match tx {
            starknet_types_rpc::DeclareTxn::V0(tx) => Self::V0(tx.into()),
            starknet_types_rpc::DeclareTxn::V1(tx) => Self::V1(tx.into()),
            starknet_types_rpc::DeclareTxn::V2(tx) => Self::V2(tx.into()),
            starknet_types_rpc::DeclareTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_types_rpc::DeclareTxnV0<Felt>> for DeclareTransactionV0 {
    fn from(tx: starknet_types_rpc::DeclareTxnV0<Felt>) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            class_hash: tx.class_hash,
        }
    }
}

impl From<starknet_types_rpc::DeclareTxnV1<Felt>> for DeclareTransactionV1 {
    fn from(tx: starknet_types_rpc::DeclareTxnV1<Felt>) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<starknet_types_rpc::DeclareTxnV2<Felt>> for DeclareTransactionV2 {
    fn from(tx: starknet_types_rpc::DeclareTxnV2<Felt>) -> Self {
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

impl From<starknet_types_rpc::DeclareTxnV3<Felt>> for DeclareTransactionV3 {
    fn from(tx: starknet_types_rpc::DeclareTxnV3<Felt>) -> Self {
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

impl From<starknet_types_rpc::DeployTxn<Felt>> for DeployTransaction {
    fn from(tx: starknet_types_rpc::DeployTxn<Felt>) -> Self {
        Self {
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl From<starknet_types_rpc::DeployAccountTxn<Felt>> for DeployAccountTransaction {
    fn from(tx: starknet_types_rpc::DeployAccountTxn<Felt>) -> Self {
        match tx {
            starknet_types_rpc::DeployAccountTxn::V1(tx) => Self::V1(tx.into()),
            starknet_types_rpc::DeployAccountTxn::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_types_rpc::DeployAccountTxnV1<Felt>> for DeployAccountTransactionV1 {
    fn from(tx: starknet_types_rpc::DeployAccountTxnV1<Felt>) -> Self {
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
impl From<starknet_types_rpc::DeployAccountTxnV3<Felt>> for DeployAccountTransactionV3 {
    fn from(tx: starknet_types_rpc::DeployAccountTxnV3<Felt>) -> Self {
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
