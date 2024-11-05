use crate::{
    broadcasted_to_blockifier::is_query, BroadcastedDeclareTransactionV0, DeclareTransaction, DeclareTransactionV0,
    DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3, DeployAccountTransaction,
    DeployAccountTransactionV1, DeployAccountTransactionV3, InvokeTransaction, InvokeTransactionV1,
    InvokeTransactionV3, Transaction, TransactionWithHash,
};
use mp_chain_config::StarknetVersion;
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeclareTransactionV1, BroadcastedDeclareTransactionV2,
    BroadcastedDeclareTransactionV3, BroadcastedDeployAccountTransaction, BroadcastedDeployAccountTransactionV1,
    BroadcastedDeployAccountTransactionV3, BroadcastedInvokeTransaction, BroadcastedInvokeTransactionV1,
    BroadcastedInvokeTransactionV3, BroadcastedTransaction,
};
use starknet_types_core::felt::Felt;

// class_hash is required for DeclareTransaction
impl TransactionWithHash {
    pub fn from_broadcasted(
        tx: BroadcastedTransaction,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = is_query(&tx);
        let transaction = match tx {
            BroadcastedTransaction::Invoke(tx) => Transaction::Invoke(tx.into()),
            BroadcastedTransaction::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
            BroadcastedTransaction::Declare(tx) => Transaction::Declare(DeclareTransaction::from_broadcasted(
                tx,
                class_hash.expect("Class hash must be provided for DeclareTransaction"),
            )),
        };
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        Self { hash, transaction }
    }
}

impl From<BroadcastedInvokeTransaction> for InvokeTransaction {
    fn from(tx: BroadcastedInvokeTransaction) -> Self {
        match tx {
            BroadcastedInvokeTransaction::V1(tx) => InvokeTransaction::V1(tx.into()),
            BroadcastedInvokeTransaction::V3(tx) => InvokeTransaction::V3(tx.into()),
        }
    }
}

impl From<BroadcastedInvokeTransactionV1> for InvokeTransactionV1 {
    fn from(tx: BroadcastedInvokeTransactionV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<BroadcastedInvokeTransactionV3> for InvokeTransactionV3 {
    fn from(tx: BroadcastedInvokeTransactionV3) -> Self {
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

impl DeclareTransaction {
    fn from_broadcasted(tx: BroadcastedDeclareTransaction, class_hash: Felt) -> Self {
        match tx {
            BroadcastedDeclareTransaction::V1(tx) => {
                DeclareTransaction::V1(DeclareTransactionV1::from_broadcasted(tx, class_hash))
            }
            BroadcastedDeclareTransaction::V2(tx) => {
                DeclareTransaction::V2(DeclareTransactionV2::from_broadcasted(tx, class_hash))
            }
            BroadcastedDeclareTransaction::V3(tx) => {
                DeclareTransaction::V3(DeclareTransactionV3::from_broadcasted(tx, class_hash))
            }
        }
    }

    pub fn from_broadcasted_declare_v0(tx: BroadcastedDeclareTransactionV0, class_hash: Felt) -> Self {
        DeclareTransaction::V0(DeclareTransactionV0::from_broadcasted_declare_v0(tx, class_hash))
    }
}

impl DeclareTransactionV0 {
    fn from_broadcasted_declare_v0(tx: BroadcastedDeclareTransactionV0, class_hash: Felt) -> Self {
        Self { sender_address: tx.sender_address, max_fee: tx.max_fee, signature: tx.signature, class_hash }
    }
}

impl DeclareTransactionV1 {
    fn from_broadcasted(tx: BroadcastedDeclareTransactionV1, class_hash: Felt) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash,
        }
    }
}

impl DeclareTransactionV2 {
    fn from_broadcasted(tx: BroadcastedDeclareTransactionV2, class_hash: Felt) -> Self {
        Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash,
        }
    }
}

impl DeclareTransactionV3 {
    fn from_broadcasted(tx: BroadcastedDeclareTransactionV3, class_hash: Felt) -> Self {
        Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash,
            resource_bounds: tx.resource_bounds.into(),
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}

impl From<BroadcastedDeployAccountTransaction> for DeployAccountTransaction {
    fn from(tx: BroadcastedDeployAccountTransaction) -> Self {
        match tx {
            BroadcastedDeployAccountTransaction::V1(tx) => DeployAccountTransaction::V1(tx.into()),
            BroadcastedDeployAccountTransaction::V3(tx) => DeployAccountTransaction::V3(tx.into()),
        }
    }
}

impl From<BroadcastedDeployAccountTransactionV1> for DeployAccountTransactionV1 {
    fn from(tx: BroadcastedDeployAccountTransactionV1) -> Self {
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

impl From<BroadcastedDeployAccountTransactionV3> for DeployAccountTransactionV3 {
    fn from(tx: BroadcastedDeployAccountTransactionV3) -> Self {
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
