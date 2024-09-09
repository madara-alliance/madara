use mp_chain_config::StarknetVersion;
use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransaction, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3, DeployAccountTransaction,
    DeployAccountTransactionV1, DeployAccountTransactionV3, InvokeTransaction, InvokeTransactionV1,
    InvokeTransactionV3, Transaction, TransactionWithHash,
};

// class_hash is required for DeclareTransaction
impl TransactionWithHash {
    pub fn from_broadcasted(
        tx: starknet_core::types::BroadcastedTransaction,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = is_query(&tx);
        let transaction: Transaction = match tx {
            starknet_core::types::BroadcastedTransaction::Invoke(tx) => Transaction::Invoke(tx.into()),
            starknet_core::types::BroadcastedTransaction::Declare(tx) => {
                Transaction::Declare(DeclareTransaction::from_broadcasted(tx, class_hash.unwrap()))
            }
            starknet_core::types::BroadcastedTransaction::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
        };
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        Self { hash, transaction }
    }
}

impl From<starknet_core::types::BroadcastedInvokeTransaction> for InvokeTransaction {
    fn from(tx: starknet_core::types::BroadcastedInvokeTransaction) -> Self {
        match tx {
            starknet_core::types::BroadcastedInvokeTransaction::V1(tx) => InvokeTransaction::V1(tx.into()),
            starknet_core::types::BroadcastedInvokeTransaction::V3(tx) => InvokeTransaction::V3(tx.into()),
        }
    }
}

impl From<starknet_core::types::BroadcastedInvokeTransactionV1> for InvokeTransactionV1 {
    fn from(tx: starknet_core::types::BroadcastedInvokeTransactionV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

impl From<starknet_core::types::BroadcastedInvokeTransactionV3> for InvokeTransactionV3 {
    fn from(tx: starknet_core::types::BroadcastedInvokeTransactionV3) -> Self {
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
    fn from_broadcasted(tx: starknet_core::types::BroadcastedDeclareTransaction, class_hash: Felt) -> Self {
        match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => {
                DeclareTransaction::V1(DeclareTransactionV1::from_broadcasted(tx, class_hash))
            }
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => {
                DeclareTransaction::V2(DeclareTransactionV2::from_broadcasted(tx, class_hash))
            }
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => {
                DeclareTransaction::V3(DeclareTransactionV3::from_broadcasted(tx, class_hash))
            }
        }
    }
}

impl DeclareTransactionV1 {
    fn from_broadcasted(tx: starknet_core::types::BroadcastedDeclareTransactionV1, class_hash: Felt) -> Self {
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
    fn from_broadcasted(tx: starknet_core::types::BroadcastedDeclareTransactionV2, class_hash: Felt) -> Self {
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
    fn from_broadcasted(tx: starknet_core::types::BroadcastedDeclareTransactionV3, class_hash: Felt) -> Self {
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

impl From<starknet_core::types::BroadcastedDeployAccountTransaction> for DeployAccountTransaction {
    fn from(tx: starknet_core::types::BroadcastedDeployAccountTransaction) -> Self {
        match tx {
            starknet_core::types::BroadcastedDeployAccountTransaction::V1(tx) => {
                DeployAccountTransaction::V1(tx.into())
            }
            starknet_core::types::BroadcastedDeployAccountTransaction::V3(tx) => {
                DeployAccountTransaction::V3(tx.into())
            }
        }
    }
}

impl From<starknet_core::types::BroadcastedDeployAccountTransactionV1> for DeployAccountTransactionV1 {
    fn from(tx: starknet_core::types::BroadcastedDeployAccountTransactionV1) -> Self {
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

impl From<starknet_core::types::BroadcastedDeployAccountTransactionV3> for DeployAccountTransactionV3 {
    fn from(tx: starknet_core::types::BroadcastedDeployAccountTransactionV3) -> Self {
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

fn is_query(tx: &starknet_core::types::BroadcastedTransaction) -> bool {
    match tx {
        starknet_core::types::BroadcastedTransaction::Invoke(tx) => match tx {
            starknet_core::types::BroadcastedInvokeTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedInvokeTransaction::V3(tx) => tx.is_query,
        },
        starknet_core::types::BroadcastedTransaction::Declare(tx) => match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => tx.is_query,
        },
        starknet_core::types::BroadcastedTransaction::DeployAccount(tx) => match tx {
            starknet_core::types::BroadcastedDeployAccountTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeployAccountTransaction::V3(tx) => tx.is_query,
        },
    }
}
