use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, InvokeTransaction, Transaction, TransactionWithHash,
};
use mp_chain_config::StarknetVersion;
use starknet_types_core::felt::Felt;

// class_hash is required for DeclareTransaction
impl TransactionWithHash {
    pub fn from_broadcasted(
        tx: mp_rpc::v0_7_1::BroadcastedTxn,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = is_query(&tx);
        let transaction: Transaction = match tx {
            mp_rpc::v0_7_1::BroadcastedTxn::Invoke(tx) => Transaction::Invoke(tx.into()),
            mp_rpc::v0_7_1::BroadcastedTxn::Declare(tx) => Transaction::Declare(DeclareTransaction::from_broadcasted(
                tx,
                class_hash.expect("Class hash must be provided for DeclareTransaction"),
            )),
            mp_rpc::v0_7_1::BroadcastedTxn::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
        };
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        Self { hash, transaction }
    }
}

impl From<mp_rpc::v0_7_1::BroadcastedInvokeTxn> for InvokeTransaction {
    fn from(tx: mp_rpc::v0_7_1::BroadcastedInvokeTxn) -> Self {
        match tx {
            mp_rpc::v0_7_1::BroadcastedInvokeTxn::V0(tx) => InvokeTransaction::V0(tx.into()),
            mp_rpc::v0_7_1::BroadcastedInvokeTxn::V1(tx) => InvokeTransaction::V1(tx.into()),
            mp_rpc::v0_7_1::BroadcastedInvokeTxn::V3(tx) => InvokeTransaction::V3(tx.into()),
            mp_rpc::v0_7_1::BroadcastedInvokeTxn::QueryV0(tx) => InvokeTransaction::V0(tx.into()),
            mp_rpc::v0_7_1::BroadcastedInvokeTxn::QueryV1(tx) => InvokeTransaction::V1(tx.into()),
            mp_rpc::v0_7_1::BroadcastedInvokeTxn::QueryV3(tx) => InvokeTransaction::V3(tx.into()),
        }
    }
}

impl DeclareTransaction {
    fn from_broadcasted(tx: mp_rpc::v0_7_1::BroadcastedDeclareTxn, class_hash: Felt) -> Self {
        match tx {
            mp_rpc::v0_7_1::BroadcastedDeclareTxn::V1(tx) | mp_rpc::v0_7_1::BroadcastedDeclareTxn::QueryV1(tx) => {
                DeclareTransaction::V1(DeclareTransactionV1::from_broadcasted(tx, class_hash))
            }
            mp_rpc::v0_7_1::BroadcastedDeclareTxn::V2(tx) | mp_rpc::v0_7_1::BroadcastedDeclareTxn::QueryV2(tx) => {
                DeclareTransaction::V2(DeclareTransactionV2::from_broadcasted(tx, class_hash))
            }
            mp_rpc::v0_7_1::BroadcastedDeclareTxn::V3(tx) | mp_rpc::v0_7_1::BroadcastedDeclareTxn::QueryV3(tx) => {
                DeclareTransaction::V3(DeclareTransactionV3::from_broadcasted(tx, class_hash))
            }
        }
    }

    pub fn from_broadcasted_declare_v0(tx: mp_rpc::admin::BroadcastedDeclareTxnV0, class_hash: Felt) -> Self {
        DeclareTransaction::V0(DeclareTransactionV0::from_broadcasted_declare_v0(tx, class_hash))
    }
}

impl DeclareTransactionV0 {
    fn from_broadcasted_declare_v0(tx: mp_rpc::admin::BroadcastedDeclareTxnV0, class_hash: Felt) -> Self {
        Self { sender_address: tx.sender_address, max_fee: tx.max_fee, signature: tx.signature, class_hash }
    }
}

impl DeclareTransactionV1 {
    fn from_broadcasted(tx: mp_rpc::v0_7_1::BroadcastedDeclareTxnV1, class_hash: Felt) -> Self {
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
    fn from_broadcasted(tx: mp_rpc::v0_7_1::BroadcastedDeclareTxnV2, class_hash: Felt) -> Self {
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
    fn from_broadcasted(tx: mp_rpc::v0_7_1::BroadcastedDeclareTxnV3, class_hash: Felt) -> Self {
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

impl From<mp_rpc::v0_7_1::BroadcastedDeployAccountTxn> for DeployAccountTransaction {
    fn from(tx: mp_rpc::v0_7_1::BroadcastedDeployAccountTxn) -> Self {
        match tx {
            mp_rpc::v0_7_1::BroadcastedDeployAccountTxn::V1(tx)
            | mp_rpc::v0_7_1::BroadcastedDeployAccountTxn::QueryV1(tx) => DeployAccountTransaction::V1(tx.into()),
            mp_rpc::v0_7_1::BroadcastedDeployAccountTxn::V3(tx)
            | mp_rpc::v0_7_1::BroadcastedDeployAccountTxn::QueryV3(tx) => DeployAccountTransaction::V3(tx.into()),
        }
    }
}

pub(crate) fn is_query(tx: &mp_rpc::v0_7_1::BroadcastedTxn) -> bool {
    use mp_rpc::v0_7_1::{BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, BroadcastedTxn};

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
