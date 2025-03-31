use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl From<Transaction> for mp_rpc::Txn {
    fn from(tx: Transaction) -> Self {
        match tx {
            Transaction::Invoke(tx) => mp_rpc::Txn::Invoke(tx.into()),
            Transaction::L1Handler(tx) => mp_rpc::Txn::L1Handler(tx.into()),
            Transaction::Declare(tx) => mp_rpc::Txn::Declare(tx.into()),
            Transaction::Deploy(tx) => mp_rpc::Txn::Deploy(tx.into()),
            Transaction::DeployAccount(tx) => mp_rpc::Txn::DeployAccount(tx.into()),
        }
    }
}

impl From<InvokeTransaction> for mp_rpc::InvokeTxn {
    fn from(tx: InvokeTransaction) -> Self {
        match tx {
            InvokeTransaction::V0(tx) => mp_rpc::InvokeTxn::V0(tx.into()),
            InvokeTransaction::V1(tx) => mp_rpc::InvokeTxn::V1(tx.into()),
            InvokeTransaction::V3(tx) => mp_rpc::InvokeTxn::V3(tx.into()),
        }
    }
}

impl From<InvokeTransactionV0> for mp_rpc::InvokeTxnV0 {
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

impl From<InvokeTransactionV1> for mp_rpc::InvokeTxnV1 {
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

impl From<InvokeTransactionV3> for mp_rpc::InvokeTxnV3 {
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

impl From<L1HandlerTransaction> for mp_rpc::L1HandlerTxn {
    fn from(tx: L1HandlerTransaction) -> Self {
        Self {
            nonce: tx.nonce,
            version: tx.version.to_hex_string(),
            function_call: mp_rpc::FunctionCall {
                calldata: tx.calldata,
                contract_address: tx.contract_address,
                entry_point_selector: tx.entry_point_selector,
            },
        }
    }
}

impl From<DeclareTransaction> for mp_rpc::DeclareTxn {
    fn from(tx: DeclareTransaction) -> Self {
        match tx {
            DeclareTransaction::V0(tx) => mp_rpc::DeclareTxn::V0(tx.into()),
            DeclareTransaction::V1(tx) => mp_rpc::DeclareTxn::V1(tx.into()),
            DeclareTransaction::V2(tx) => mp_rpc::DeclareTxn::V2(tx.into()),
            DeclareTransaction::V3(tx) => mp_rpc::DeclareTxn::V3(tx.into()),
        }
    }
}

impl From<DeclareTransactionV0> for mp_rpc::DeclareTxnV0 {
    fn from(tx: DeclareTransactionV0) -> Self {
        Self {
            class_hash: tx.class_hash,
            max_fee: tx.max_fee,
            sender_address: tx.sender_address,
            signature: tx.signature,
        }
    }
}

impl From<DeclareTransactionV1> for mp_rpc::DeclareTxnV1 {
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

impl From<DeclareTransactionV2> for mp_rpc::DeclareTxnV2 {
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

impl From<DeclareTransactionV3> for mp_rpc::DeclareTxnV3 {
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

impl From<DeployTransaction> for mp_rpc::DeployTxn {
    fn from(tx: DeployTransaction) -> Self {
        Self {
            class_hash: tx.class_hash,
            constructor_calldata: tx.constructor_calldata,
            contract_address_salt: tx.contract_address_salt,
            version: tx.version,
        }
    }
}

impl From<DeployAccountTransaction> for mp_rpc::DeployAccountTxn {
    fn from(tx: DeployAccountTransaction) -> Self {
        match tx {
            DeployAccountTransaction::V1(tx) => mp_rpc::DeployAccountTxn::V1(tx.into()),
            DeployAccountTransaction::V3(tx) => mp_rpc::DeployAccountTxn::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTransactionV1> for mp_rpc::DeployAccountTxnV1 {
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

impl From<DeployAccountTransactionV3> for mp_rpc::DeployAccountTxnV3 {
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
