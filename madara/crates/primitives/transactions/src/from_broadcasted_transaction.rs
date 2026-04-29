use crate::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, InvokeTransaction, Transaction, TransactionWithHash,
};
use mp_chain_config::StarknetVersion;
use starknet_types_core::felt::Felt;

// class_hash is required for DeclareTransaction
impl TransactionWithHash {
    pub fn from_broadcasted_v0_7(
        tx: mp_rpc::v0_7_1::BroadcastedTxn,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = tx.is_query();
        let transaction: Transaction = match tx {
            mp_rpc::v0_7_1::BroadcastedTxn::Invoke(tx) => Transaction::Invoke(tx.into()),
            mp_rpc::v0_7_1::BroadcastedTxn::Declare(tx) => {
                Transaction::Declare(DeclareTransaction::from_broadcasted_v0_7(
                    tx,
                    class_hash.expect("Class hash must be provided for DeclareTransaction"),
                ))
            }
            mp_rpc::v0_7_1::BroadcastedTxn::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
        };
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        Self { hash, transaction }
    }

    pub fn from_broadcasted_v0_8(
        tx: mp_rpc::v0_8_1::BroadcastedTxn,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = tx.is_query();
        let transaction: Transaction = match tx {
            mp_rpc::v0_8_1::BroadcastedTxn::Invoke(tx) => Transaction::Invoke(tx.into()),
            mp_rpc::v0_8_1::BroadcastedTxn::Declare(tx) => {
                Transaction::Declare(DeclareTransaction::from_broadcasted_v0_8(
                    tx,
                    class_hash.expect("Class hash must be provided for DeclareTransaction"),
                ))
            }
            mp_rpc::v0_8_1::BroadcastedTxn::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
        };
        let hash = transaction.compute_hash(chain_id, starknet_version, is_query);
        Self { hash, transaction }
    }

    pub fn from_broadcasted_v0_10_2(
        tx: mp_rpc::v0_10_2::BroadcastedTxn,
        chain_id: Felt,
        starknet_version: StarknetVersion,
        class_hash: Option<Felt>,
    ) -> Self {
        let is_query = tx.is_query();
        let transaction: Transaction = match tx {
            mp_rpc::v0_10_2::BroadcastedTxn::Invoke(tx) => Transaction::Invoke(tx.into()),
            mp_rpc::v0_10_2::BroadcastedTxn::Declare(tx) => {
                Transaction::Declare(DeclareTransaction::from_broadcasted_v0_8(
                    tx,
                    class_hash.expect("Class hash must be provided for DeclareTransaction"),
                ))
            }
            mp_rpc::v0_10_2::BroadcastedTxn::DeployAccount(tx) => Transaction::DeployAccount(tx.into()),
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

impl From<mp_rpc::v0_8_1::BroadcastedInvokeTxn> for InvokeTransaction {
    fn from(tx: mp_rpc::v0_8_1::BroadcastedInvokeTxn) -> Self {
        match tx {
            mp_rpc::v0_8_1::BroadcastedInvokeTxn::V0(tx) => InvokeTransaction::V0(tx.into()),
            mp_rpc::v0_8_1::BroadcastedInvokeTxn::V1(tx) => InvokeTransaction::V1(tx.into()),
            mp_rpc::v0_8_1::BroadcastedInvokeTxn::V3(tx) => InvokeTransaction::V3(tx.into()),
            mp_rpc::v0_8_1::BroadcastedInvokeTxn::QueryV0(tx) => InvokeTransaction::V0(tx.into()),
            mp_rpc::v0_8_1::BroadcastedInvokeTxn::QueryV1(tx) => InvokeTransaction::V1(tx.into()),
            mp_rpc::v0_8_1::BroadcastedInvokeTxn::QueryV3(tx) => InvokeTransaction::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::v0_10_2::BroadcastedInvokeTxn> for InvokeTransaction {
    fn from(tx: mp_rpc::v0_10_2::BroadcastedInvokeTxn) -> Self {
        match tx {
            mp_rpc::v0_10_2::BroadcastedInvokeTxn::V0(tx) => InvokeTransaction::V0(tx.into()),
            mp_rpc::v0_10_2::BroadcastedInvokeTxn::V1(tx) => InvokeTransaction::V1(tx.into()),
            mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(tx) => InvokeTransaction::V3(tx.into()),
            mp_rpc::v0_10_2::BroadcastedInvokeTxn::QueryV0(tx) => InvokeTransaction::V0(tx.into()),
            mp_rpc::v0_10_2::BroadcastedInvokeTxn::QueryV1(tx) => InvokeTransaction::V1(tx.into()),
            mp_rpc::v0_10_2::BroadcastedInvokeTxn::QueryV3(tx) => InvokeTransaction::V3(tx.into()),
        }
    }
}

impl From<mp_rpc::v0_10_2::BroadcastedInvokeTxnV3> for crate::InvokeTransactionV3 {
    fn from(tx: mp_rpc::v0_10_2::BroadcastedInvokeTxnV3) -> Self {
        let mut invoke: crate::InvokeTransactionV3 = tx.inner.into();
        invoke.proof_facts =
            tx.proof_facts.or_else(|| tx.proof.map(|proof| proof.into_iter().map(Felt::from).collect()));
        invoke
    }
}

impl DeclareTransaction {
    fn from_broadcasted_v0_7(tx: mp_rpc::v0_7_1::BroadcastedDeclareTxn, class_hash: Felt) -> Self {
        match tx {
            mp_rpc::v0_7_1::BroadcastedDeclareTxn::V1(tx) | mp_rpc::v0_7_1::BroadcastedDeclareTxn::QueryV1(tx) => {
                DeclareTransaction::V1(DeclareTransactionV1::from_broadcasted(tx, class_hash))
            }
            mp_rpc::v0_7_1::BroadcastedDeclareTxn::V2(tx) | mp_rpc::v0_7_1::BroadcastedDeclareTxn::QueryV2(tx) => {
                DeclareTransaction::V2(DeclareTransactionV2::from_broadcasted(tx, class_hash))
            }
            mp_rpc::v0_7_1::BroadcastedDeclareTxn::V3(tx) | mp_rpc::v0_7_1::BroadcastedDeclareTxn::QueryV3(tx) => {
                DeclareTransaction::V3(DeclareTransactionV3::from_broadcasted_v0_7(tx, class_hash))
            }
        }
    }
    fn from_broadcasted_v0_8(tx: mp_rpc::v0_8_1::BroadcastedDeclareTxn, class_hash: Felt) -> Self {
        match tx {
            mp_rpc::v0_8_1::BroadcastedDeclareTxn::V1(tx) | mp_rpc::v0_8_1::BroadcastedDeclareTxn::QueryV1(tx) => {
                DeclareTransaction::V1(DeclareTransactionV1::from_broadcasted(tx, class_hash))
            }
            mp_rpc::v0_8_1::BroadcastedDeclareTxn::V2(tx) | mp_rpc::v0_8_1::BroadcastedDeclareTxn::QueryV2(tx) => {
                DeclareTransaction::V2(DeclareTransactionV2::from_broadcasted(tx, class_hash))
            }
            mp_rpc::v0_8_1::BroadcastedDeclareTxn::V3(tx) | mp_rpc::v0_8_1::BroadcastedDeclareTxn::QueryV3(tx) => {
                DeclareTransaction::V3(DeclareTransactionV3::from_broadcasted_v0_8(tx, class_hash))
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
    fn from_broadcasted_v0_7(tx: mp_rpc::v0_7_1::BroadcastedDeclareTxnV3, class_hash: Felt) -> Self {
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
    fn from_broadcasted_v0_8(tx: mp_rpc::v0_8_1::BroadcastedDeclareTxnV3, class_hash: Felt) -> Self {
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

impl From<mp_rpc::v0_8_1::BroadcastedDeployAccountTxn> for DeployAccountTransaction {
    fn from(tx: mp_rpc::v0_8_1::BroadcastedDeployAccountTxn) -> Self {
        match tx {
            mp_rpc::v0_8_1::BroadcastedDeployAccountTxn::V1(tx)
            | mp_rpc::v0_8_1::BroadcastedDeployAccountTxn::QueryV1(tx) => DeployAccountTransaction::V1(tx.into()),
            mp_rpc::v0_8_1::BroadcastedDeployAccountTxn::V3(tx)
            | mp_rpc::v0_8_1::BroadcastedDeployAccountTxn::QueryV3(tx) => DeployAccountTransaction::V3(tx.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TEST_CHAIN_ID;
    use mp_chain_config::StarknetVersion;
    use mp_rpc::v0_10_0::{DaMode, InvokeTxnV3, ResourceBounds, ResourceBoundsMapping};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct BroadcastedInvokeV3HashVector {
        label: String,
        chain_id: String,
        source_url: String,
        transaction_hash: String,
        transaction: mp_rpc::v0_10_2::BroadcastedInvokeTxnV3,
    }

    fn parse_chain_id(chain_id: &str) -> Felt {
        match chain_id {
            "SN_SEPOLIA" => TEST_CHAIN_ID,
            other => panic!("unsupported chain id in broadcasted invoke vector: {other}"),
        }
    }

    fn sample_broadcasted_invoke_v3() -> mp_rpc::v0_10_2::BroadcastedInvokeTxnV3 {
        mp_rpc::v0_10_2::BroadcastedInvokeTxnV3 {
            inner: InvokeTxnV3 {
                sender_address: Felt::ONE,
                calldata: vec![Felt::TWO, Felt::THREE].into(),
                signature: vec![Felt::from(4_u64)].into(),
                nonce: Felt::from(5_u64),
                resource_bounds: ResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 2 },
                    l2_gas: ResourceBounds { max_amount: 3, max_price_per_unit: 4 },
                    l1_data_gas: ResourceBounds { max_amount: 5, max_price_per_unit: 6 },
                },
                tip: 7,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DaMode::L1,
                fee_data_availability_mode: DaMode::L1,
            },
            proof: Some(vec![11, 12]),
            proof_facts: None,
        }
    }

    #[rstest::rstest]
    #[case(None, Some(vec![11, 12]), vec![Felt::from(11_u64), Felt::from(12_u64)])]
    #[case(
        Some(vec![
            Felt::from_hex_unchecked("0x50524f4f4630"),
            Felt::from_hex_unchecked("0x5649525455414c5f534e4f53"),
        ]),
        Some(vec![11, 12]),
        vec![
            Felt::from_hex_unchecked("0x50524f4f4630"),
            Felt::from_hex_unchecked("0x5649525455414c5f534e4f53"),
        ],
    )]
    fn v0_10_2_broadcasted_invoke_v3_preserves_query_hashing_and_proof_fields(
        #[case] proof_facts: Option<Vec<Felt>>,
        #[case] proof: Option<Vec<u64>>,
        #[case] expected_proof_facts: Vec<Felt>,
    ) {
        let mut base = sample_broadcasted_invoke_v3();
        base.proof_facts = proof_facts;
        base.proof = proof;

        let regular: mp_rpc::v0_8_1::BroadcastedTxn =
            mp_rpc::v0_10_2::BroadcastedTxn::Invoke(mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(base.clone())).into();
        let query: mp_rpc::v0_8_1::BroadcastedTxn =
            mp_rpc::v0_10_2::BroadcastedTxn::Invoke(mp_rpc::v0_10_2::BroadcastedInvokeTxn::QueryV3(base.clone()))
                .into();

        assert!(!regular.is_query());
        assert!(query.is_query());

        let regular_hash =
            TransactionWithHash::from_broadcasted_v0_8(regular, TEST_CHAIN_ID, StarknetVersion::LATEST, None).hash;
        let query_hash =
            TransactionWithHash::from_broadcasted_v0_8(query, TEST_CHAIN_ID, StarknetVersion::LATEST, None).hash;
        assert_ne!(regular_hash, query_hash, "query transactions must use the simulate-version hash offset");

        let InvokeTransaction::V3(invoke) = InvokeTransaction::from(mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(base))
        else {
            panic!("expected invoke v3");
        };
        assert_eq!(invoke.proof_facts, Some(expected_proof_facts));
    }

    #[test]
    fn replay_invoke_v3_with_proof_facts_matches_sepolia_hash() {
        let vectors: Vec<BroadcastedInvokeV3HashVector> =
            serde_json::from_str(include_str!("test_data/broadcasted_invoke_v3_hash_vectors.json")).unwrap();

        for vector in vectors {
            let tx = mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(vector.transaction);
            let hash = Transaction::Invoke(InvokeTransaction::from(tx)).compute_hash(
                parse_chain_id(&vector.chain_id),
                StarknetVersion::LATEST,
                false,
            );

            assert_eq!(
                hash,
                Felt::from_hex_unchecked(&vector.transaction_hash),
                "hash mismatch for {} ({})",
                vector.label,
                vector.source_url
            );
        }
    }
}
