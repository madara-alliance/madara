use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBoundsMapping, Transaction,
};

impl From<Transaction> for starknet_core::types::Transaction {
    fn from(tx: Transaction) -> Self {
        match tx {
            Transaction::Invoke(tx) => Self::Invoke(tx.into()),
            Transaction::L1Handler(tx) => Self::L1Handler(tx.into()),
            Transaction::Declare(tx) => Self::Declare(tx.into()),
            Transaction::Deploy(tx) => Self::Deploy(tx.into()),
            Transaction::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}

impl From<InvokeTransaction> for starknet_core::types::InvokeTransaction {
    fn from(tx: InvokeTransaction) -> Self {
        match tx {
            InvokeTransaction::V0(tx) => Self::V0(tx.into()),
            InvokeTransaction::V1(tx) => Self::V1(tx.into()),
            InvokeTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<InvokeTransactionV0> for starknet_core::types::InvokeTransactionV0 {
    fn from(tx: InvokeTransactionV0) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            max_fee: tx.max_fee,
            signature: tx.signature,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<InvokeTransactionV1> for starknet_core::types::InvokeTransactionV1 {
    fn from(tx: InvokeTransactionV1) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<InvokeTransactionV3> for starknet_core::types::InvokeTransactionV3 {
    fn from(tx: InvokeTransactionV3) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
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

impl From<L1HandlerTransaction> for starknet_core::types::L1HandlerTransaction {
    fn from(tx: L1HandlerTransaction) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            version: tx.version,
            nonce: tx.nonce,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

impl From<DeclareTransaction> for starknet_core::types::DeclareTransaction {
    fn from(tx: DeclareTransaction) -> Self {
        match tx {
            DeclareTransaction::V0(tx) => Self::V0(tx.into()),
            DeclareTransaction::V1(tx) => Self::V1(tx.into()),
            DeclareTransaction::V2(tx) => Self::V2(tx.into()),
            DeclareTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<DeclareTransactionV0> for starknet_core::types::DeclareTransactionV0 {
    fn from(tx: DeclareTransactionV0) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            class_hash: tx.class_hash,
        }
    }
}

impl From<DeclareTransactionV1> for starknet_core::types::DeclareTransactionV1 {
    fn from(tx: DeclareTransactionV1) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<DeclareTransactionV2> for starknet_core::types::DeclareTransactionV2 {
    fn from(tx: DeclareTransactionV2) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

impl From<DeclareTransactionV3> for starknet_core::types::DeclareTransactionV3 {
    fn from(tx: DeclareTransactionV3) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
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

impl From<DeployTransaction> for starknet_core::types::DeployTransaction {
    fn from(tx: DeployTransaction) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl From<DeployAccountTransaction> for starknet_core::types::DeployAccountTransaction {
    fn from(tx: DeployAccountTransaction) -> Self {
        match tx {
            DeployAccountTransaction::V1(tx) => Self::V1(tx.into()),
            DeployAccountTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTransactionV1> for starknet_core::types::DeployAccountTransactionV1 {
    fn from(tx: DeployAccountTransactionV1) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl From<DeployAccountTransactionV3> for starknet_core::types::DeployAccountTransactionV3 {
    fn from(tx: DeployAccountTransactionV3) -> Self {
        Self {
            transaction_hash: tx.transaction_hash,
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

impl From<ResourceBoundsMapping> for starknet_core::types::ResourceBoundsMapping {
    fn from(resource: ResourceBoundsMapping) -> Self {
        Self {
            l1_gas: starknet_core::types::ResourceBounds {
                max_amount: resource.l1_gas.max_amount,
                max_price_per_unit: resource.l1_gas.max_price_per_unit,
            },
            l2_gas: starknet_core::types::ResourceBounds {
                max_amount: resource.l2_gas.max_amount,
                max_price_per_unit: resource.l2_gas.max_price_per_unit,
            },
        }
    }
}

impl From<DataAvailabilityMode> for starknet_core::types::DataAvailabilityMode {
    fn from(da_mode: DataAvailabilityMode) -> Self {
        match da_mode {
            DataAvailabilityMode::L1 => Self::L1,
            DataAvailabilityMode::L2 => Self::L2,
        }
    }
}
