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

    #[test]
    fn v0_10_2_broadcasted_invoke_v3_preserves_query_hashing_and_proof_fields() {
        let cases = [
            (None, Some(vec![11, 12]), vec![Felt::from(11_u64), Felt::from(12_u64)]),
            (
                Some(vec![
                    Felt::from_hex_unchecked("0x50524f4f4630"),
                    Felt::from_hex_unchecked("0x5649525455414c5f534e4f53"),
                ]),
                Some(vec![11, 12]),
                vec![
                    Felt::from_hex_unchecked("0x50524f4f4630"),
                    Felt::from_hex_unchecked("0x5649525455414c5f534e4f53"),
                ],
            ),
        ];

        for (proof_facts, proof, expected_proof_facts) in cases {
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

            let InvokeTransaction::V3(invoke) =
                InvokeTransaction::from(mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(base))
            else {
                panic!("expected invoke v3");
            };
            assert_eq!(invoke.proof_facts, Some(expected_proof_facts));
        }
    }

    #[test]
    fn replay_invoke_v3_with_proof_facts_matches_sepolia_hash() {
        let tx = mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(mp_rpc::v0_10_2::BroadcastedInvokeTxnV3 {
            inner: InvokeTxnV3 {
                sender_address: Felt::from_hex_unchecked(
                    "0x7ae2c9e63c7ee60372bacb2e86db4f958b2709cd8e0637788933389a315ee45",
                ),
                calldata: vec![
                    Felt::from_hex_unchecked("0x1"),
                    Felt::from_hex_unchecked("0x75a180e18e56da1b1cae181c92a288f586f5fe22c18df21cf97886f1e4b316c"),
                    Felt::from_hex_unchecked("0x25933a01f93ed7abacc8133bf5dc66ad2f5722e0a879d4dede30dabc3881047"),
                    Felt::from_hex_unchecked("0x31"),
                    Felt::from_hex_unchecked("0x2"),
                    Felt::from_hex_unchecked("0x192b9a24f91be6275777dc355fe0b12f2b7db4da6d9217f0e9788f957f4c432"),
                    Felt::from_hex_unchecked("0x34cc13b274446654ca3233ed2c1620d4c5d1d32fd20b47146a3371064bdc57d"),
                    Felt::from_hex_unchecked("0xe"),
                    Felt::from_hex_unchecked("0x75a180e18e56da1b1cae181c92a288f586f5fe22c18df21cf97886f1e4b316c"),
                    Felt::from_hex_unchecked("0x753e4db34afd1b8e094e6be0c9b56532"),
                    Felt::from_hex_unchecked("0x1"),
                    Felt::from_hex_unchecked("0x69cbc5b4"),
                    Felt::from_hex_unchecked("0x1"),
                    Felt::from_hex_unchecked("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
                    Felt::from_hex_unchecked("0x219209e083275171774dab1df80982e9df2096516f06319c5c6d71ae0a8480c"),
                    Felt::from_hex_unchecked("0x3"),
                    Felt::from_hex_unchecked("0xd894af9ed2bdede33675049ae5285df000c44258a2250b84a9c3bed0d7c233"),
                    Felt::from_hex_unchecked("0x2b5e3af16b1880000"),
                    Felt::from_hex_unchecked("0x0"),
                    Felt::from_hex_unchecked("0x2"),
                    Felt::from_hex_unchecked("0x57342f44df9d83f56c97a2420a6fb48c2af2335306a9497f2078cfbd871ca88"),
                    Felt::from_hex_unchecked("0x653d0f0853626c851db54ce891b473a3582427901075892c53cf32304dfa01c"),
                    Felt::from_hex_unchecked("0xd894af9ed2bdede33675049ae5285df000c44258a2250b84a9c3bed0d7c233"),
                    Felt::from_hex_unchecked("0x246333a752c1ac637ff1591c5c885e27d56060d241a29aad8475072da0777db"),
                    Felt::from_hex_unchecked("0x1b"),
                    Felt::from_hex_unchecked("0x6"),
                    Felt::from_hex_unchecked("0x2"),
                    Felt::from_hex_unchecked("0x192b9a24f91be6275777dc355fe0b12f2b7db4da6d9217f0e9788f957f4c432"),
                    Felt::from_hex_unchecked("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
                    Felt::from_hex_unchecked("0x2b5e3af16b1880000"),
                    Felt::from_hex_unchecked("0x6"),
                    Felt::from_hex_unchecked("0x192b9a24f91be6275777dc355fe0b12f2b7db4da6d9217f0e9788f957f4c432"),
                    Felt::from_hex_unchecked("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
                    Felt::from_hex_unchecked("0x2b5e3af16b1880000"),
                    Felt::from_hex_unchecked("0x0"),
                    Felt::from_hex_unchecked("0x4bb9def5919624c678cf4dc7be7a8126b6c2f652e66f4d8e385df6553577e4a"),
                    Felt::from_hex_unchecked("0x1"),
                    Felt::from_hex_unchecked("0x4a49bfa61eebae3b93669737b08ed8036a5ad2a58189aa47d105ed91961795"),
                    Felt::from_hex_unchecked("0x8"),
                    Felt::from_hex_unchecked("0x4df18929b50cb8b8466814fe6551d912f11ea800fe64990dac98ec5ff09f9d0"),
                    Felt::from_hex_unchecked("0x4a49bfa61eebae3b93669737b08ed8036a5ad2a58189aa47d105ed91961795"),
                    Felt::from_hex_unchecked("0x3"),
                    Felt::from_hex_unchecked("0x75a180e18e56da1b1cae181c92a288f586f5fe22c18df21cf97886f1e4b316c"),
                    Felt::from_hex_unchecked("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
                    Felt::from_hex_unchecked("0x854f5e1b599fa540"),
                    Felt::from_hex_unchecked("0x5"),
                    Felt::from_hex_unchecked("0x2fbf66c1dd8c556f8f9ee8852669513a9559385194da39ff0e33ed38586fe47"),
                    Felt::from_hex_unchecked("0x52c9965dae6c21b3bf648dd6e70e9eccef5b7a26698cca29cf54aa452a4b871"),
                    Felt::from_hex_unchecked("0x4bc0bec4e1ab98cd413364f7d81657dd92812c1ec32924acb8ce64608c07e99"),
                    Felt::from_hex_unchecked("0x75a180e18e56da1b1cae181c92a288f586f5fe22c18df21cf97886f1e4b316c"),
                    Felt::from_hex_unchecked("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
                    Felt::from_hex_unchecked("0x854f5e1b599fa540"),
                    Felt::from_hex_unchecked("0x0"),
                ]
                .into(),
                signature: vec![
                    Felt::from_hex_unchecked("0x5cb857710c5e9e2e7dde66c509ecd392b7e3328a483e9ee6326376f44284c26"),
                    Felt::from_hex_unchecked("0x3f2b88aa300bad32a23137f371051b1601a08eff44bbae0beed86ee7ea43e38"),
                ]
                .into(),
                nonce: Felt::from_hex_unchecked("0x4357"),
                resource_bounds: ResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0x50475c75f82c },
                    l2_gas: ResourceBounds { max_amount: 0x15752a00, max_price_per_unit: 0x2cb417800 },
                    l1_data_gas: ResourceBounds { max_amount: 0x1d4c, max_price_per_unit: 0x158ca },
                },
                tip: 0,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DaMode::L1,
                fee_data_availability_mode: DaMode::L1,
            },
            proof: None,
            proof_facts: Some(vec![
                Felt::from_hex_unchecked("0x50524f4f4630"),
                Felt::from_hex_unchecked("0x5649525455414c5f534e4f53"),
                Felt::from_hex_unchecked("0x3e98c2d7703b03a7edb73ed7f075f97f1dcbaa8f717cdf6e1a57bf058265473"),
                Felt::from_hex_unchecked("0x5649525455414c5f534e4f5330"),
                Felt::from_hex_unchecked("0x7e1879"),
                Felt::from_hex_unchecked("0x4e39d28d70db4afd1edd02917f5804e329c236b01fbd72857804fa2065d6721"),
                Felt::from_hex_unchecked("0x1b9900f77ff5923183a7795fcfbb54ed76917bc1ddd4160cc77fa96e36cf8c5"),
                Felt::from_hex_unchecked("0x1"),
                Felt::from_hex_unchecked("0x52b4f445e5f429d76c67d1616b89193e03fde104327f30c73cc293a2fc1cdff"),
            ]),
        });

        let hash = Transaction::Invoke(InvokeTransaction::from(tx)).compute_hash(
            TEST_CHAIN_ID,
            StarknetVersion::LATEST,
            false,
        );

        assert_eq!(hash, Felt::from_hex_unchecked("0x20b72b09e352ffc746cb1163463fc980183c4b85b41d171ffef51aba106f3b1"));
    }
}
