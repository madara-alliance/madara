use crate::{
    BroadcastedDeclareTransactionV0, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1,
    DeclareTransactionV2, DeclareTransactionV3, DeployAccountTransaction, InvokeTransaction, Transaction,
    TransactionWithHash,
};
use mp_chain_config::StarknetVersion;
use starknet_types_core::felt::Felt;

// class_hash is required for DeclareTransaction
impl TransactionWithHash {
    pub fn from_broadcasted(
        tx: starknet_types_rpc::BroadcastedTxn<Felt>,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = is_query(&tx);
        let transaction: Transaction = match tx {
            starknet_types_rpc::BroadcastedTxn::Invoke(tx) => Transaction::Invoke(tx.into()),
            starknet_types_rpc::BroadcastedTxn::Declare(tx) => {
                Transaction::Declare(DeclareTransaction::from_broadcasted(
                    tx,
                    class_hash.expect("Class hash must be provided for DeclareTransaction"),
                ))
            }
            starknet_types_rpc::BroadcastedTxn::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
        };
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        Self { hash, transaction }
    }
}

impl From<starknet_types_rpc::BroadcastedInvokeTxn<Felt>> for InvokeTransaction {
    fn from(tx: starknet_types_rpc::BroadcastedInvokeTxn<Felt>) -> Self {
        match tx {
            starknet_types_rpc::BroadcastedInvokeTxn::V0(tx) => InvokeTransaction::V0(tx.into()),
            starknet_types_rpc::BroadcastedInvokeTxn::V1(tx) => InvokeTransaction::V1(tx.into()),
            starknet_types_rpc::BroadcastedInvokeTxn::V3(tx) => InvokeTransaction::V3(tx.into()),
            starknet_types_rpc::BroadcastedInvokeTxn::QueryV0(tx) => InvokeTransaction::V0(tx.into()),
            starknet_types_rpc::BroadcastedInvokeTxn::QueryV1(tx) => InvokeTransaction::V1(tx.into()),
            starknet_types_rpc::BroadcastedInvokeTxn::QueryV3(tx) => InvokeTransaction::V3(tx.into()),
        }
    }
}

impl DeclareTransaction {
    fn from_broadcasted(tx: starknet_types_rpc::BroadcastedDeclareTxn<Felt>, class_hash: Felt) -> Self {
        match tx {
            starknet_types_rpc::BroadcastedDeclareTxn::V1(tx)
            | starknet_types_rpc::BroadcastedDeclareTxn::QueryV1(tx) => {
                DeclareTransaction::V1(DeclareTransactionV1::from_broadcasted(tx, class_hash))
            }
            starknet_types_rpc::BroadcastedDeclareTxn::V2(tx)
            | starknet_types_rpc::BroadcastedDeclareTxn::QueryV2(tx) => {
                DeclareTransaction::V2(DeclareTransactionV2::from_broadcasted(tx, class_hash))
            }
            starknet_types_rpc::BroadcastedDeclareTxn::V3(tx)
            | starknet_types_rpc::BroadcastedDeclareTxn::QueryV3(tx) => {
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
    fn from_broadcasted(tx: starknet_types_rpc::BroadcastedDeclareTxnV1<Felt>, class_hash: Felt) -> Self {
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
    fn from_broadcasted(tx: starknet_types_rpc::BroadcastedDeclareTxnV2<Felt>, class_hash: Felt) -> Self {
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
    fn from_broadcasted(tx: starknet_types_rpc::BroadcastedDeclareTxnV3<Felt>, class_hash: Felt) -> Self {
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

impl From<starknet_types_rpc::BroadcastedDeployAccountTxn<Felt>> for DeployAccountTransaction {
    fn from(tx: starknet_types_rpc::BroadcastedDeployAccountTxn<Felt>) -> Self {
        match tx {
            starknet_types_rpc::BroadcastedDeployAccountTxn::V1(tx)
            | starknet_types_rpc::BroadcastedDeployAccountTxn::QueryV1(tx) => DeployAccountTransaction::V1(tx.into()),
            starknet_types_rpc::BroadcastedDeployAccountTxn::V3(tx)
            | starknet_types_rpc::BroadcastedDeployAccountTxn::QueryV3(tx) => DeployAccountTransaction::V3(tx.into()),
        }
    }
}

pub(crate) fn is_query(tx: &starknet_types_rpc::BroadcastedTxn<Felt>) -> bool {
    use starknet_types_rpc::{
        BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, BroadcastedTxn,
    };

    match tx {
        BroadcastedTxn::Invoke(tx) => matches!(
            tx,
            BroadcastedInvokeTxn::QueryV0(_) | BroadcastedInvokeTxn::QueryV1(_) | BroadcastedInvokeTxn::QueryV3(_)
        ),
        BroadcastedTxn::Declare(tx) => matches!(
            tx,
            BroadcastedDeclareTxn::QueryV1(_) | BroadcastedDeclareTxn::QueryV2(_) | BroadcastedDeclareTxn::QueryV3(_)
        ),
        BroadcastedTxn::DeployAccount(tx) => {
            matches!(tx, BroadcastedDeployAccountTxn::QueryV1(_) | BroadcastedDeployAccountTxn::QueryV3(_))
        }
    }
}
