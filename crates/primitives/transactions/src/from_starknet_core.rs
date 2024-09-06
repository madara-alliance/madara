use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl From<starknet_core::types::Transaction> for Transaction {
    fn from(tx: starknet_core::types::Transaction) -> Self {
        match tx {
            starknet_core::types::Transaction::Invoke(tx) => Self::Invoke(tx.into()),
            starknet_core::types::Transaction::L1Handler(tx) => Self::L1Handler(tx.into()),
            starknet_core::types::Transaction::Declare(tx) => Self::Declare(tx.into()),
            starknet_core::types::Transaction::Deploy(tx) => Self::Deploy(tx.into()),
            starknet_core::types::Transaction::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}

impl From<starknet_core::types::InvokeTransaction> for InvokeTransaction {
    fn from(tx: starknet_core::types::InvokeTransaction) -> Self {
        match tx {
            starknet_core::types::InvokeTransaction::V0(tx) => Self::V0(tx.into()),
            starknet_core::types::InvokeTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_core::types::InvokeTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_core::types::InvokeTransactionV0> for InvokeTransactionV0 {
    fn from(tx: starknet_core::types::InvokeTransactionV0) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<starknet_core::types::InvokeTransactionV1> for InvokeTransactionV1 {
    fn from(tx: starknet_core::types::InvokeTransactionV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<starknet_core::types::InvokeTransactionV3> for InvokeTransactionV3 {
    fn from(tx: starknet_core::types::InvokeTransactionV3) -> Self {
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

impl From<starknet_core::types::L1HandlerTransaction> for L1HandlerTransaction {
    fn from(tx: starknet_core::types::L1HandlerTransaction) -> Self {
        Self {
            version: tx.version,
            nonce: tx.nonce,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<starknet_core::types::DeclareTransaction> for DeclareTransaction {
    fn from(tx: starknet_core::types::DeclareTransaction) -> Self {
        match tx {
            starknet_core::types::DeclareTransaction::V0(tx) => Self::V0(tx.into()),
            starknet_core::types::DeclareTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_core::types::DeclareTransaction::V2(tx) => Self::V2(tx.into()),
            starknet_core::types::DeclareTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_core::types::DeclareTransactionV0> for DeclareTransactionV0 {
    fn from(tx: starknet_core::types::DeclareTransactionV0) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            class_hash: tx.class_hash,
        }
    }
}

impl From<starknet_core::types::DeclareTransactionV1> for DeclareTransactionV1 {
    fn from(tx: starknet_core::types::DeclareTransactionV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<starknet_core::types::DeclareTransactionV2> for DeclareTransactionV2 {
    fn from(tx: starknet_core::types::DeclareTransactionV2) -> Self {
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

impl From<starknet_core::types::DeclareTransactionV3> for DeclareTransactionV3 {
    fn from(tx: starknet_core::types::DeclareTransactionV3) -> Self {
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

impl From<starknet_core::types::DeployTransaction> for DeployTransaction {
    fn from(tx: starknet_core::types::DeployTransaction) -> Self {
        Self {
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl From<starknet_core::types::DeployAccountTransaction> for DeployAccountTransaction {
    fn from(tx: starknet_core::types::DeployAccountTransaction) -> Self {
        match tx {
            starknet_core::types::DeployAccountTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_core::types::DeployAccountTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_core::types::DeployAccountTransactionV1> for DeployAccountTransactionV1 {
    fn from(tx: starknet_core::types::DeployAccountTransactionV1) -> Self {
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
impl From<starknet_core::types::DeployAccountTransactionV3> for DeployAccountTransactionV3 {
    fn from(tx: starknet_core::types::DeployAccountTransactionV3) -> Self {
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
