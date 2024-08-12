use mp_convert::felt_to_u64;

use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction,
};

impl From<starknet_api::transaction::Transaction> for Transaction {
    fn from(value: starknet_api::transaction::Transaction) -> Self {
        match value {
            starknet_api::transaction::Transaction::Declare(tx) => Self::Declare(tx.into()),
            starknet_api::transaction::Transaction::Deploy(tx) => Self::Deploy(tx.into()),
            starknet_api::transaction::Transaction::DeployAccount(tx) => Self::DeployAccount(tx.into()),
            starknet_api::transaction::Transaction::Invoke(tx) => Self::Invoke(tx.into()),
            starknet_api::transaction::Transaction::L1Handler(tx) => Self::L1Handler(tx.into()),
        }
    }
}

impl From<starknet_api::transaction::DeclareTransaction> for DeclareTransaction {
    fn from(value: starknet_api::transaction::DeclareTransaction) -> Self {
        match value {
            starknet_api::transaction::DeclareTransaction::V0(tx) => Self::V0(declare_v0v1_into_v0(tx)),
            starknet_api::transaction::DeclareTransaction::V1(tx) => Self::V1(declare_v0v1_into_v1(tx)),
            starknet_api::transaction::DeclareTransaction::V2(tx) => Self::V2(tx.into()),
            starknet_api::transaction::DeclareTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

fn declare_v0v1_into_v0(tx: starknet_api::transaction::DeclareTransactionV0V1) -> DeclareTransactionV0 {
    DeclareTransactionV0 {
        sender_address: **tx.sender_address,
        max_fee: (*tx.max_fee).into(),
        signature: tx.signature.0,
        class_hash: *tx.class_hash,
    }
}
fn declare_v0v1_into_v1(tx: starknet_api::transaction::DeclareTransactionV0V1) -> DeclareTransactionV1 {
    DeclareTransactionV1 {
        sender_address: **tx.sender_address,
        max_fee: (*tx.max_fee).into(),
        signature: tx.signature.0,
        class_hash: *tx.class_hash,
        nonce: *tx.nonce,
    }
}

impl From<starknet_api::transaction::DeclareTransactionV2> for DeclareTransactionV2 {
    fn from(value: starknet_api::transaction::DeclareTransactionV2) -> Self {
        Self {
            sender_address: **value.sender_address,
            max_fee: (*value.max_fee).into(),
            signature: value.signature.0,
            class_hash: *value.class_hash,
            nonce: *value.nonce,
            compiled_class_hash: value.compiled_class_hash.0,
        }
    }
}

impl From<starknet_api::transaction::DeclareTransactionV3> for DeclareTransactionV3 {
    fn from(value: starknet_api::transaction::DeclareTransactionV3) -> Self {
        Self {
            sender_address: **value.sender_address,
            signature: value.signature.0,
            class_hash: *value.class_hash,
            nonce: *value.nonce,
            compiled_class_hash: value.compiled_class_hash.0,
            resource_bounds: value.resource_bounds.into(),
            tip: *value.tip,
            paymaster_data: value.paymaster_data.0,
            account_deployment_data: value.account_deployment_data.0,
            nonce_data_availability_mode: value.nonce_data_availability_mode.into(),
            fee_data_availability_mode: value.fee_data_availability_mode.into(),
        }
    }
}

impl From<starknet_api::transaction::DeployTransaction> for DeployTransaction {
    fn from(value: starknet_api::transaction::DeployTransaction) -> Self {
        Self {
            version: *value.version,
            contract_address_salt: value.contract_address_salt.0,
            constructor_calldata: value.constructor_calldata.0.to_vec(),
            class_hash: *value.class_hash,
        }
    }
}

impl From<starknet_api::transaction::DeployAccountTransaction> for DeployAccountTransaction {
    fn from(value: starknet_api::transaction::DeployAccountTransaction) -> Self {
        match value {
            starknet_api::transaction::DeployAccountTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_api::transaction::DeployAccountTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_api::transaction::DeployAccountTransactionV1> for DeployAccountTransactionV1 {
    fn from(value: starknet_api::transaction::DeployAccountTransactionV1) -> Self {
        Self {
            max_fee: value.max_fee.into(),
            signature: value.signature.0,
            nonce: *value.nonce,
            contract_address_salt: value.contract_address_salt.0,
            constructor_calldata: value.constructor_calldata.0.to_vec(),
            class_hash: *value.class_hash,
        }
    }
}

impl From<starknet_api::transaction::DeployAccountTransactionV3> for DeployAccountTransactionV3 {
    fn from(value: starknet_api::transaction::DeployAccountTransactionV3) -> Self {
        Self {
            signature: value.signature.0,
            nonce: *value.nonce,
            contract_address_salt: value.contract_address_salt.0,
            constructor_calldata: value.constructor_calldata.0.to_vec(),
            class_hash: *value.class_hash,
            resource_bounds: value.resource_bounds.into(),
            tip: *value.tip,
            paymaster_data: value.paymaster_data.0,
            nonce_data_availability_mode: value.nonce_data_availability_mode.into(),
            fee_data_availability_mode: value.fee_data_availability_mode.into(),
        }
    }
}

impl From<starknet_api::transaction::InvokeTransaction> for InvokeTransaction {
    fn from(value: starknet_api::transaction::InvokeTransaction) -> Self {
        match value {
            starknet_api::transaction::InvokeTransaction::V0(tx) => Self::V0(tx.into()),
            starknet_api::transaction::InvokeTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_api::transaction::InvokeTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<starknet_api::transaction::InvokeTransactionV0> for InvokeTransactionV0 {
    fn from(value: starknet_api::transaction::InvokeTransactionV0) -> Self {
        Self {
            max_fee: value.max_fee.into(),
            signature: value.signature.0,
            contract_address: **value.contract_address,
            entry_point_selector: value.entry_point_selector.0,
            calldata: value.calldata.0.to_vec(),
        }
    }
}

impl From<starknet_api::transaction::InvokeTransactionV1> for InvokeTransactionV1 {
    fn from(value: starknet_api::transaction::InvokeTransactionV1) -> Self {
        Self {
            sender_address: **value.sender_address,
            calldata: value.calldata.0.to_vec(),
            max_fee: value.max_fee.into(),
            signature: value.signature.0,
            nonce: *value.nonce,
        }
    }
}

impl From<starknet_api::transaction::InvokeTransactionV3> for InvokeTransactionV3 {
    fn from(value: starknet_api::transaction::InvokeTransactionV3) -> Self {
        Self {
            sender_address: **value.sender_address,
            calldata: value.calldata.0.to_vec(),
            signature: value.signature.0,
            nonce: *value.nonce,
            resource_bounds: value.resource_bounds.into(),
            tip: *value.tip,
            paymaster_data: value.paymaster_data.0,
            account_deployment_data: value.account_deployment_data.0,
            nonce_data_availability_mode: value.nonce_data_availability_mode.into(),
            fee_data_availability_mode: value.fee_data_availability_mode.into(),
        }
    }
}

impl From<starknet_api::transaction::L1HandlerTransaction> for L1HandlerTransaction {
    fn from(value: starknet_api::transaction::L1HandlerTransaction) -> Self {
        Self {
            version: value.version.0,
            nonce: felt_to_u64(&value.nonce).unwrap(),
            contract_address: **value.contract_address,
            entry_point_selector: value.entry_point_selector.0,
            calldata: value.calldata.0.to_vec(),
        }
    }
}

impl From<starknet_api::data_availability::DataAvailabilityMode> for DataAvailabilityMode {
    fn from(value: starknet_api::data_availability::DataAvailabilityMode) -> Self {
        match value {
            starknet_api::data_availability::DataAvailabilityMode::L1 => Self::L1,
            starknet_api::data_availability::DataAvailabilityMode::L2 => Self::L2,
        }
    }
}

impl From<starknet_api::transaction::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(mut value: starknet_api::transaction::ResourceBoundsMapping) -> Self {
        ResourceBoundsMapping {
            l1_gas: value.0.remove(&starknet_api::transaction::Resource::L1Gas).unwrap_or_default().into(),
            l2_gas: value.0.remove(&starknet_api::transaction::Resource::L2Gas).unwrap_or_default().into(),
        }
    }
}

impl From<starknet_api::transaction::ResourceBounds> for ResourceBounds {
    fn from(value: starknet_api::transaction::ResourceBounds) -> Self {
        ResourceBounds { max_amount: value.max_amount, max_price_per_unit: value.max_price_per_unit }
    }
}
