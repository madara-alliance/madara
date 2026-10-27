use super::*;
use blockifier::execution::syscalls::committed_data::CommittedDataSet;
use mc_mempool::{Mempool, MempoolConfig};
use mc_submit_tx::{TransactionValidator, TransactionValidatorConfig};
use mp_transactions::{validated::TxTimestamp, IntoStarknetApiExt};

fn signed_transaction(backend: &MadaraBackend, account: &DevnetPredeployedContract, kind: &str) -> BroadcastedTxn {
    let mut encoded = serde_json::to_value(signed_two_call_tx(backend, account)).unwrap();
    encoded["type"] = kind.into();
    match kind {
        "DECLARE" => {
            let class: starknet_core::types::contract::SierraClass =
                serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
            let class: mp_class::FlattenedSierraClass = class.flatten().unwrap().into();
            encoded["compiled_class_hash"] = serde_json::json!(class.compile_to_casm_with_hashes().unwrap().blake_hash);
            encoded["contract_class"] = serde_json::to_value(class).unwrap();
            encoded.as_object_mut().unwrap().remove("calldata");
        }
        "DEPLOY_ACCOUNT" => {
            for field in ["sender_address", "calldata", "account_deployment_data"] {
                encoded.as_object_mut().unwrap().remove(field);
            }
            encoded["class_hash"] = serde_json::json!(account.class_hash);
            encoded["contract_address_salt"] = serde_json::json!(Felt::ZERO);
            encoded["constructor_calldata"] = serde_json::json!([account.pubkey]);
        }
        "INVOKE" => {}
        _ => unreachable!(),
    }
    let tx: BroadcastedTxn = serde_json::from_value(encoded.clone()).unwrap();
    let hash = tx
        .into_validated_tx(
            backend.chain_config().chain_id.to_felt(),
            backend.chain_config().latest_protocol_version,
            TxTimestamp::now(),
        )
        .unwrap()
        .hash;
    let signature = account.secret.sign(&hash).unwrap();
    encoded["signature"] = serde_json::json!([signature.r, signature.s]);
    serde_json::from_value(encoded).unwrap()
}

#[rstest::rstest]
#[case::invoke("INVOKE", false, false)]
#[case::declare("DECLARE", false, false)]
#[case::deploy_account("DEPLOY_ACCOUNT", false, false)]
#[case::invalid_signature("INVOKE", true, false)]
#[case::query_only("INVOKE", false, true)]
#[tokio::test]
async fn combined_rpc_validates_and_persists_exact_admission(
    #[case] kind: &str,
    #[case] invalid_signature: bool,
    #[case] query: bool,
) {
    let backend = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_devnet()));
    backend.set_l1_gas_quote_for_testing();
    let mut genesis = mc_devnet::ChainGenesisDescription::base_config().unwrap();
    let base_deployments = genesis.deployed_contracts.clone();
    let keys = genesis.add_devnet_contracts(1).unwrap();
    let account = &keys.0[0];
    if kind == "DEPLOY_ACCOUNT" {
        // Keep its funding and declared class, but let the real deployment validator run the constructor.
        genesis.deployed_contracts = base_deployments;
    }
    genesis.build_and_store(&backend).await.unwrap();
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let validator =
        Arc::new(TransactionValidator::new(mempool.clone(), backend.clone(), TransactionValidatorConfig::default()));
    let mut rpc = crate::Starknet::new(
        backend.clone(),
        validator.clone(),
        validator,
        Default::default(),
        None,
        mp_utils::service::ServiceContext::new_for_testing(),
    );
    rpc.ctx.service_remove(MadaraServiceId::L2Sync);
    rpc.committed_data_transaction_submission = true;
    let tx = signed_transaction(&backend, account, kind);
    let expected = tx
        .clone()
        .into_validated_tx(
            backend.chain_config().chain_id.to_felt(),
            backend.chain_config().latest_protocol_version,
            TxTimestamp::now(),
        )
        .unwrap();
    let mut encoded = serde_json::to_value(tx).unwrap();
    if invalid_signature {
        encoded["signature"] = serde_json::json!(["0x0", "0x0"]);
    }
    if query {
        encoded["version"] = "0x100000000000000000000000000000003".into();
    }
    let datasets: Vec<_> = [42_u32, 43]
        .into_iter()
        .map(|value| {
            let entries = vec![(Felt::ZERO, Felt::from(value))];
            let root = CommittedDataSet::new(entries.clone()).unwrap().root();
            (root, entries)
        })
        .collect();
    let imports: Vec<_> =
        datasets.iter().map(|(root, entries)| serde_json::json!({"root":root,"entries":entries})).collect();
    let request = serde_json::json!({"jsonrpc":"2.0","id":1,
        "method":"madara_V0_1_0_importCommittedDataAndSubmitTransaction","params":[imports,encoded]})
    .to_string();
    let module = crate::rpc_api_committed_data(&rpc).unwrap();
    let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    if invalid_signature || query {
        assert_eq!(response["error"]["code"], if query { -32602 } else { 55 }, "{response}");
        assert!(mempool.is_empty().await);
        assert_eq!(backend.get_saved_mempool_transactions().count(), 0);
        assert!(backend.db.recovery_dependencies(expected.hash).unwrap().is_empty());
    } else {
        assert_eq!(response["result"], serde_json::json!({"transaction_hash":expected.hash}), "{response}");
        assert!(mempool.is_transaction_in_mempool(&expected.hash));
        let saved = backend.get_saved_mempool_transactions().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(saved.len(), 1);
        let mut expected = expected;
        expected.arrived_at = saved[0].arrived_at;
        assert_eq!(saved, vec![expected.clone()]);
        let dependencies: Vec<_> =
            datasets.iter().map(|(root, _)| backend.recovery_reference(None, *root).unwrap()).collect();
        assert_eq!(backend.db.recovery_dependencies(expected.hash).unwrap(), dependencies);
    }
    for (root, _) in datasets {
        // Query-only is rejected before import; validation rejection retains the durable data.
        assert_eq!(backend.committed_data_witness(root, Felt::ZERO).unwrap().is_some(), !query);
    }
}

#[tokio::test]
async fn full_backlog_rejects_import_before_writing_but_allows_exact_retry() {
    use mc_db::committed_data_recovery::MAX_PENDING_BYTES;
    let (backend, rpc) = crate::test_utils::rpc_test_setup();
    let module = crate::rpc_api_committed_data(&rpc).unwrap();
    let entries = vec![(Felt::ONE, Felt::TWO)];
    let root = CommittedDataSet::new(entries.clone()).unwrap().root();
    let call = |root, entries| {
        serde_json::json!({"jsonrpc":"2.0","id":1,
        "method":"madara_V0_1_0_importCommittedDataSet","params":[root,entries]})
        .to_string()
    };
    let request = call(root, entries.clone());
    let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&response).unwrap().get("error").is_none(), "{response}");
    let id = backend.recovery_reference(None, root).unwrap();
    let db = backend.db.inner_db();
    let cf = db.cf_handle("committed_data_recovery").unwrap();
    db.put_cf(&cf, b"bytes", MAX_PENDING_BYTES.to_le_bytes()).unwrap();
    let next_entries = vec![(Felt::ONE, Felt::THREE)];
    let next = CommittedDataSet::new(next_entries.clone()).unwrap().root();
    let (response, _) = module.raw_json_request(&call(next, next_entries), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["error"]["code"], 500, "{response}");
    assert!(backend.db.get_committed_data_count(next).unwrap().is_none());
    assert!(backend.recovery_reference(None, next).is_err());
    assert_eq!(backend.db.pending_recovery_records(10).unwrap(), vec![id.clone()]);
    let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&response).unwrap().get("error").is_none(), "{response}");
    assert_eq!(backend.recovery_reference(None, root).unwrap(), id);
    assert_eq!(backend.db.recovery_pending_bytes().unwrap(), MAX_PENDING_BYTES);
}
