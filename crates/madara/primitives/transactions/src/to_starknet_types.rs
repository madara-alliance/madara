use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl From<Transaction> for starknet_types_rpc::Txn<Felt> {
    fn from(tx: Transaction) -> Self {
        match tx {
            Transaction::Invoke(tx) => starknet_types_rpc::Txn::Invoke(tx.into()),
            Transaction::L1Handler(tx) => starknet_types_rpc::Txn::L1Handler(tx.into()),
            Transaction::Declare(tx) => starknet_types_rpc::Txn::Declare(tx.into()),
            Transaction::Deploy(tx) => starknet_types_rpc::Txn::Deploy(tx.into()),
            Transaction::DeployAccount(tx) => starknet_types_rpc::Txn::DeployAccount(tx.into()),
        }
    }
}

impl From<InvokeTransaction> for starknet_types_rpc::InvokeTxn<Felt> {
    fn from(tx: InvokeTransaction) -> Self {
        match tx {
            InvokeTransaction::V0(tx) => starknet_types_rpc::InvokeTxn::V0(tx.into()),
            InvokeTransaction::V1(tx) => starknet_types_rpc::InvokeTxn::V1(tx.into()),
            InvokeTransaction::V3(tx) => starknet_types_rpc::InvokeTxn::V3(tx.into()),
        }
    }
}

impl From<InvokeTransactionV0> for starknet_types_rpc::InvokeTxnV0<Felt> {
    fn from(tx: InvokeTransactionV0) -> Self {
        Self {
            calldata: tx.calldata,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            max_fee: tx.max_fee,
            signature: tx.signature,
        }
    }
}

impl From<InvokeTransactionV1> for starknet_types_rpc::InvokeTxnV1<Felt> {
    fn from(tx: InvokeTransactionV1) -> Self {
        Self {
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            nonce: tx.nonce,
            sender_address: tx.sender_address,
            signature: tx.signature,
        }
    }
}

impl From<InvokeTransactionV3> for starknet_types_rpc::InvokeTxnV3<Felt> {
    fn from(tx: InvokeTransactionV3) -> Self {
        Self {
            account_deployment_data: tx.account_deployment_data,
            calldata: tx.calldata,
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            nonce: tx.nonce,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds.into(),
            sender_address: tx.sender_address,
            signature: tx.signature,
            tip: tx.tip,
        }
    }
}

impl From<L1HandlerTransaction> for starknet_types_rpc::L1HandlerTxn<Felt> {
    fn from(tx: L1HandlerTransaction) -> Self {
        Self {
            nonce: tx.nonce,
            version: tx.version.to_hex_string(),
            function_call: starknet_types_rpc::FunctionCall {
                calldata: tx.calldata,
                contract_address: tx.contract_address,
                entry_point_selector: tx.entry_point_selector,
            },
        }
    }
}

impl From<DeclareTransaction> for starknet_types_rpc::DeclareTxn<Felt> {
    fn from(tx: DeclareTransaction) -> Self {
        match tx {
            DeclareTransaction::V0(tx) => starknet_types_rpc::DeclareTxn::V0(tx.into()),
            DeclareTransaction::V1(tx) => starknet_types_rpc::DeclareTxn::V1(tx.into()),
            DeclareTransaction::V2(tx) => starknet_types_rpc::DeclareTxn::V2(tx.into()),
            DeclareTransaction::V3(tx) => starknet_types_rpc::DeclareTxn::V3(tx.into()),
        }
    }
}

impl From<DeclareTransactionV0> for starknet_types_rpc::DeclareTxnV0<Felt> {
    fn from(tx: DeclareTransactionV0) -> Self {
        Self {
            class_hash: tx.class_hash,
            max_fee: tx.max_fee,
            sender_address: tx.sender_address,
            signature: tx.signature,
        }
    }
}

impl From<DeclareTransactionV1> for starknet_types_rpc::DeclareTxnV1<Felt> {
    fn from(tx: DeclareTransactionV1) -> Self {
        Self {
            class_hash: tx.class_hash,
            max_fee: tx.max_fee,
            nonce: tx.nonce,
            sender_address: tx.sender_address,
            signature: tx.signature,
        }
    }
}

impl From<DeclareTransactionV2> for starknet_types_rpc::DeclareTxnV2<Felt> {
    fn from(tx: DeclareTransactionV2) -> Self {
        Self {
            class_hash: tx.class_hash,
            compiled_class_hash: tx.compiled_class_hash,
            max_fee: tx.max_fee,
            nonce: tx.nonce,
            sender_address: tx.sender_address,
            signature: tx.signature,
        }
    }
}

impl From<DeclareTransactionV3> for starknet_types_rpc::DeclareTxnV3<Felt> {
    fn from(tx: DeclareTransactionV3) -> Self {
        Self {
            account_deployment_data: tx.account_deployment_data,
            class_hash: tx.class_hash,
            compiled_class_hash: tx.compiled_class_hash,
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            nonce: tx.nonce,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds.into(),
            sender_address: tx.sender_address,
            signature: tx.signature,
            tip: tx.tip,
        }
    }
}

impl From<DeployTransaction> for starknet_types_rpc::DeployTxn<Felt> {
    fn from(tx: DeployTransaction) -> Self {
        Self {
            class_hash: tx.class_hash,
            constructor_calldata: tx.constructor_calldata,
            contract_address_salt: tx.contract_address_salt,
            version: tx.version,
        }
    }
}

impl From<DeployAccountTransaction> for starknet_types_rpc::DeployAccountTxn<Felt> {
    fn from(tx: DeployAccountTransaction) -> Self {
        match tx {
            DeployAccountTransaction::V1(tx) => starknet_types_rpc::DeployAccountTxn::V1(tx.into()),
            DeployAccountTransaction::V3(tx) => starknet_types_rpc::DeployAccountTxn::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTransactionV1> for starknet_types_rpc::DeployAccountTxnV1<Felt> {
    fn from(tx: DeployAccountTransactionV1) -> Self {
        Self {
            class_hash: tx.class_hash,
            constructor_calldata: tx.constructor_calldata,
            contract_address_salt: tx.contract_address_salt,
            max_fee: tx.max_fee,
            nonce: tx.nonce,
            signature: tx.signature,
        }
    }
}

impl From<DeployAccountTransactionV3> for starknet_types_rpc::DeployAccountTxnV3<Felt> {
    fn from(tx: DeployAccountTransactionV3) -> Self {
        Self {
            class_hash: tx.class_hash,
            constructor_calldata: tx.constructor_calldata,
            contract_address_salt: tx.contract_address_salt,
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            nonce: tx.nonce,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds.into(),
            signature: tx.signature,
            tip: tx.tip,
        }
    }
}
