use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    Transaction, TransactionWithHash,
};

impl From<TransactionWithHash> for starknet_core::types::Transaction {
    fn from(tx: TransactionWithHash) -> Self {
        let hash = tx.hash;
        tx.transaction.to_core(hash)
    }
}

impl Transaction {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::Transaction {
        match self {
            Transaction::Invoke(tx) => starknet_core::types::Transaction::Invoke(tx.to_core(hash)),
            Transaction::L1Handler(tx) => starknet_core::types::Transaction::L1Handler(tx.to_core(hash)),
            Transaction::Declare(tx) => starknet_core::types::Transaction::Declare(tx.to_core(hash)),
            Transaction::Deploy(tx) => starknet_core::types::Transaction::Deploy(tx.to_core(hash)),
            Transaction::DeployAccount(tx) => starknet_core::types::Transaction::DeployAccount(tx.to_core(hash)),
        }
    }

    pub fn build_core_tx(
        self,
        chain_id: Felt,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> starknet_core::types::Transaction {
        let hash = self.compute_hash(chain_id, offset_version, block_number);
        self.to_core(hash)
    }
}

impl InvokeTransaction {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::InvokeTransaction {
        match self {
            InvokeTransaction::V0(tx) => starknet_core::types::InvokeTransaction::V0(tx.to_core(hash)),
            InvokeTransaction::V1(tx) => starknet_core::types::InvokeTransaction::V1(tx.to_core(hash)),
            InvokeTransaction::V3(tx) => starknet_core::types::InvokeTransaction::V3(tx.to_core(hash)),
        }
    }
}

impl InvokeTransactionV0 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::InvokeTransactionV0 {
        starknet_core::types::InvokeTransactionV0 {
            transaction_hash: hash,
            max_fee: self.max_fee,
            signature: self.signature,
            contract_address: self.contract_address,
            entry_point_selector: self.entry_point_selector,
            calldata: self.calldata,
        }
    }
}

impl InvokeTransactionV1 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::InvokeTransactionV1 {
        starknet_core::types::InvokeTransactionV1 {
            transaction_hash: hash,
            sender_address: self.sender_address,
            calldata: self.calldata,
            max_fee: self.max_fee,
            signature: self.signature,
            nonce: self.nonce,
        }
    }
}

impl InvokeTransactionV3 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::InvokeTransactionV3 {
        starknet_core::types::InvokeTransactionV3 {
            transaction_hash: hash,
            sender_address: self.sender_address,
            calldata: self.calldata,
            signature: self.signature,
            nonce: self.nonce,
            resource_bounds: self.resource_bounds.into(),
            tip: self.tip,
            paymaster_data: self.paymaster_data,
            account_deployment_data: self.account_deployment_data,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
        }
    }
}

impl L1HandlerTransaction {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::L1HandlerTransaction {
        starknet_core::types::L1HandlerTransaction {
            transaction_hash: hash,
            version: self.version,
            nonce: self.nonce,
            contract_address: self.contract_address,
            entry_point_selector: self.entry_point_selector,
            calldata: self.calldata,
        }
    }
}

impl DeclareTransaction {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeclareTransaction {
        match self {
            DeclareTransaction::V0(tx) => starknet_core::types::DeclareTransaction::V0(tx.to_core(hash)),
            DeclareTransaction::V1(tx) => starknet_core::types::DeclareTransaction::V1(tx.to_core(hash)),
            DeclareTransaction::V2(tx) => starknet_core::types::DeclareTransaction::V2(tx.to_core(hash)),
            DeclareTransaction::V3(tx) => starknet_core::types::DeclareTransaction::V3(tx.to_core(hash)),
        }
    }
}

impl DeclareTransactionV0 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeclareTransactionV0 {
        starknet_core::types::DeclareTransactionV0 {
            transaction_hash: hash,
            sender_address: self.sender_address,
            max_fee: self.max_fee,
            signature: self.signature,
            class_hash: self.class_hash,
        }
    }
}

impl DeclareTransactionV1 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeclareTransactionV1 {
        starknet_core::types::DeclareTransactionV1 {
            transaction_hash: hash,
            sender_address: self.sender_address,
            max_fee: self.max_fee,
            signature: self.signature,
            nonce: self.nonce,
            class_hash: self.class_hash,
        }
    }
}

impl DeclareTransactionV2 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeclareTransactionV2 {
        starknet_core::types::DeclareTransactionV2 {
            transaction_hash: hash,
            sender_address: self.sender_address,
            compiled_class_hash: self.compiled_class_hash,
            max_fee: self.max_fee,
            signature: self.signature,
            nonce: self.nonce,
            class_hash: self.class_hash,
        }
    }
}

impl DeclareTransactionV3 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeclareTransactionV3 {
        starknet_core::types::DeclareTransactionV3 {
            transaction_hash: hash,
            sender_address: self.sender_address,
            compiled_class_hash: self.compiled_class_hash,
            signature: self.signature,
            nonce: self.nonce,
            class_hash: self.class_hash,
            resource_bounds: self.resource_bounds.into(),
            tip: self.tip,
            paymaster_data: self.paymaster_data,
            account_deployment_data: self.account_deployment_data,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
        }
    }
}

impl DeployTransaction {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeployTransaction {
        starknet_core::types::DeployTransaction {
            transaction_hash: hash,
            version: self.version,
            contract_address_salt: self.contract_address_salt,
            constructor_calldata: self.constructor_calldata,
            class_hash: self.class_hash,
        }
    }
}

impl DeployAccountTransaction {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeployAccountTransaction {
        match self {
            DeployAccountTransaction::V1(tx) => starknet_core::types::DeployAccountTransaction::V1(tx.to_core(hash)),
            DeployAccountTransaction::V3(tx) => starknet_core::types::DeployAccountTransaction::V3(tx.to_core(hash)),
        }
    }
}

impl DeployAccountTransactionV1 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeployAccountTransactionV1 {
        starknet_core::types::DeployAccountTransactionV1 {
            transaction_hash: hash,
            max_fee: self.max_fee,
            signature: self.signature,
            nonce: self.nonce,
            contract_address_salt: self.contract_address_salt,
            constructor_calldata: self.constructor_calldata,
            class_hash: self.class_hash,
        }
    }
}

impl DeployAccountTransactionV3 {
    pub fn to_core(self, hash: Felt) -> starknet_core::types::DeployAccountTransactionV3 {
        starknet_core::types::DeployAccountTransactionV3 {
            transaction_hash: hash,
            signature: self.signature,
            nonce: self.nonce,
            contract_address_salt: self.contract_address_salt,
            constructor_calldata: self.constructor_calldata,
            class_hash: self.class_hash,
            resource_bounds: self.resource_bounds.into(),
            tip: self.tip,
            paymaster_data: self.paymaster_data,
            nonce_data_availability_mode: self.nonce_data_availability_mode.into(),
            fee_data_availability_mode: self.fee_data_availability_mode.into(),
        }
    }
}
