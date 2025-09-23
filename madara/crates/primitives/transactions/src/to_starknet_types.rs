use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction,
};

impl Transaction {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::Txn {
        match self {
            Transaction::Invoke(tx) => mp_rpc::v0_7_1::Txn::Invoke(tx.to_rpc_v0_7()),
            Transaction::L1Handler(tx) => mp_rpc::v0_7_1::Txn::L1Handler(tx.to_rpc_v0_7()),
            Transaction::Declare(tx) => mp_rpc::v0_7_1::Txn::Declare(tx.to_rpc_v0_7()),
            Transaction::Deploy(tx) => mp_rpc::v0_7_1::Txn::Deploy(tx.to_rpc_v0_7()),
            Transaction::DeployAccount(tx) => mp_rpc::v0_7_1::Txn::DeployAccount(tx.to_rpc_v0_7()),
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::Txn {
        match self {
            Transaction::Invoke(tx) => mp_rpc::v0_8_1::Txn::Invoke(tx.to_rpc_v0_8()),
            Transaction::L1Handler(tx) => mp_rpc::v0_8_1::Txn::L1Handler(tx.to_rpc_v0_7()),
            Transaction::Declare(tx) => mp_rpc::v0_8_1::Txn::Declare(tx.to_rpc_v0_8()),
            Transaction::Deploy(tx) => mp_rpc::v0_8_1::Txn::Deploy(tx.to_rpc_v0_7()),
            Transaction::DeployAccount(tx) => mp_rpc::v0_8_1::Txn::DeployAccount(tx.to_rpc_v0_8()),
        }
    }
}

impl InvokeTransaction {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::InvokeTxn {
        match self {
            InvokeTransaction::V0(tx) => mp_rpc::v0_7_1::InvokeTxn::V0(tx.to_rpc_v0_7()),
            InvokeTransaction::V1(tx) => mp_rpc::v0_7_1::InvokeTxn::V1(tx.to_rpc_v0_7()),
            InvokeTransaction::V3(tx) => mp_rpc::v0_7_1::InvokeTxn::V3(tx.to_rpc_v0_7()),
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::InvokeTxn {
        match self {
            InvokeTransaction::V0(tx) => mp_rpc::v0_8_1::InvokeTxn::V0(tx.to_rpc_v0_7()),
            InvokeTransaction::V1(tx) => mp_rpc::v0_8_1::InvokeTxn::V1(tx.to_rpc_v0_7()),
            InvokeTransaction::V3(tx) => mp_rpc::v0_8_1::InvokeTxn::V3(tx.to_rpc_v0_8()),
        }
    }
}

impl InvokeTransactionV0 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::InvokeTxnV0 {
        mp_rpc::v0_7_1::InvokeTxnV0 {
            calldata: self.calldata,
            contract_address: self.contract_address,
            entry_point_selector: self.entry_point_selector,
            max_fee: self.max_fee,
            signature: self.signature,
        }
    }
}

impl InvokeTransactionV1 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::InvokeTxnV1 {
        mp_rpc::v0_7_1::InvokeTxnV1 {
            calldata: self.calldata,
            max_fee: self.max_fee,
            nonce: self.nonce,
            sender_address: self.sender_address,
            signature: self.signature,
        }
    }
}

impl InvokeTransactionV3 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::InvokeTxnV3 {
        mp_rpc::v0_7_1::InvokeTxnV3 {
            account_deployment_data: self.account_deployment_data,
            calldata: self.calldata,
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
            nonce: self.nonce,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds.into(),
            sender_address: self.sender_address,
            signature: self.signature,
            tip: self.tip,
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::InvokeTxnV3 {
        mp_rpc::v0_8_1::InvokeTxnV3 {
            account_deployment_data: self.account_deployment_data,
            calldata: self.calldata,
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
            nonce: self.nonce,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds.into(),
            sender_address: self.sender_address,
            signature: self.signature,
            tip: self.tip,
        }
    }
}

impl L1HandlerTransaction {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::L1HandlerTxn {
        mp_rpc::v0_7_1::L1HandlerTxn {
            nonce: self.nonce,
            version: self.version.to_hex_string(),
            function_call: mp_rpc::v0_7_1::FunctionCall {
                calldata: self.calldata,
                contract_address: self.contract_address,
                entry_point_selector: self.entry_point_selector,
            },
        }
    }
}

impl DeclareTransaction {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeclareTxn {
        match self {
            DeclareTransaction::V0(tx) => mp_rpc::v0_7_1::DeclareTxn::V0(tx.to_rpc_v0_7()),
            DeclareTransaction::V1(tx) => mp_rpc::v0_7_1::DeclareTxn::V1(tx.to_rpc_v0_7()),
            DeclareTransaction::V2(tx) => mp_rpc::v0_7_1::DeclareTxn::V2(tx.to_rpc_v0_7()),
            DeclareTransaction::V3(tx) => mp_rpc::v0_7_1::DeclareTxn::V3(tx.to_rpc_v0_7()),
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::DeclareTxn {
        match self {
            DeclareTransaction::V0(tx) => mp_rpc::v0_8_1::DeclareTxn::V0(tx.to_rpc_v0_7()),
            DeclareTransaction::V1(tx) => mp_rpc::v0_8_1::DeclareTxn::V1(tx.to_rpc_v0_7()),
            DeclareTransaction::V2(tx) => mp_rpc::v0_8_1::DeclareTxn::V2(tx.to_rpc_v0_7()),
            DeclareTransaction::V3(tx) => mp_rpc::v0_8_1::DeclareTxn::V3(tx.to_rpc_v0_8()),
        }
    }
}

impl DeclareTransactionV0 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeclareTxnV0 {
        mp_rpc::v0_7_1::DeclareTxnV0 {
            class_hash: self.class_hash,
            max_fee: self.max_fee,
            sender_address: self.sender_address,
            signature: self.signature,
        }
    }
}

impl DeclareTransactionV1 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeclareTxnV1 {
        mp_rpc::v0_7_1::DeclareTxnV1 {
            class_hash: self.class_hash,
            max_fee: self.max_fee,
            nonce: self.nonce,
            sender_address: self.sender_address,
            signature: self.signature,
        }
    }
}

impl DeclareTransactionV2 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeclareTxnV2 {
        mp_rpc::v0_7_1::DeclareTxnV2 {
            class_hash: self.class_hash,
            compiled_class_hash: self.compiled_class_hash,
            max_fee: self.max_fee,
            nonce: self.nonce,
            sender_address: self.sender_address,
            signature: self.signature,
        }
    }
}

impl DeclareTransactionV3 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeclareTxnV3 {
        mp_rpc::v0_7_1::DeclareTxnV3 {
            account_deployment_data: self.account_deployment_data,
            class_hash: self.class_hash,
            compiled_class_hash: self.compiled_class_hash,
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
            nonce: self.nonce,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds.into(),
            sender_address: self.sender_address,
            signature: self.signature,
            tip: self.tip,
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::DeclareTxnV3 {
        mp_rpc::v0_8_1::DeclareTxnV3 {
            account_deployment_data: self.account_deployment_data,
            class_hash: self.class_hash,
            compiled_class_hash: self.compiled_class_hash,
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
            nonce: self.nonce,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds.into(),
            sender_address: self.sender_address,
            signature: self.signature,
            tip: self.tip,
        }
    }
}

impl DeployTransaction {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeployTxn {
        mp_rpc::v0_7_1::DeployTxn {
            class_hash: self.class_hash,
            constructor_calldata: self.constructor_calldata,
            contract_address_salt: self.contract_address_salt,
            version: self.version,
        }
    }
}

impl DeployAccountTransaction {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeployAccountTxn {
        match self {
            DeployAccountTransaction::V1(tx) => mp_rpc::v0_7_1::DeployAccountTxn::V1(tx.to_rpc_v0_7()),
            DeployAccountTransaction::V3(tx) => mp_rpc::v0_7_1::DeployAccountTxn::V3(tx.to_rpc_v0_7()),
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::DeployAccountTxn {
        match self {
            DeployAccountTransaction::V1(tx) => mp_rpc::v0_8_1::DeployAccountTxn::V1(tx.to_rpc_v0_7()),
            DeployAccountTransaction::V3(tx) => mp_rpc::v0_8_1::DeployAccountTxn::V3(tx.to_rpc_v0_8()),
        }
    }
}

impl DeployAccountTransactionV1 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeployAccountTxnV1 {
        mp_rpc::v0_7_1::DeployAccountTxnV1 {
            class_hash: self.class_hash,
            constructor_calldata: self.constructor_calldata,
            contract_address_salt: self.contract_address_salt,
            max_fee: self.max_fee,
            nonce: self.nonce,
            signature: self.signature,
        }
    }
}

impl DeployAccountTransactionV3 {
    pub fn to_rpc_v0_7(self) -> mp_rpc::v0_7_1::DeployAccountTxnV3 {
        mp_rpc::v0_7_1::DeployAccountTxnV3 {
            class_hash: self.class_hash,
            constructor_calldata: self.constructor_calldata,
            contract_address_salt: self.contract_address_salt,
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
            nonce: self.nonce,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds.into(),
            signature: self.signature,
            tip: self.tip,
        }
    }
    pub fn to_rpc_v0_8(self) -> mp_rpc::v0_8_1::DeployAccountTxnV3 {
        mp_rpc::v0_8_1::DeployAccountTxnV3 {
            class_hash: self.class_hash,
            constructor_calldata: self.constructor_calldata,
            contract_address_salt: self.contract_address_salt,
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
            nonce: self.nonce,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds.into(),
            signature: self.signature,
            tip: self.tip,
        }
    }
}
