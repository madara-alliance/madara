use super::*;
use mc_db::{MadaraBackend, MadaraStorageRead};
use mc_devnet::{Call, DevnetPredeployedContract, Multicall, Selector};
use mc_submit_tx::{SubmitTransaction, SubmitTransactionError};
use mp_convert::ToFelt;
use mp_rpc::v0_10_2::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Account multicalls, including Oracle root + final-price calls, are one signed INVOKE.
/// Use two fee-token calls here so the RPC routing test does not depend on an Oracle fixture.
fn signed_two_call_tx(backend: &MadaraBackend, account: &DevnetPredeployedContract) -> BroadcastedTxn {
    use mp_rpc::v0_9_0::{BroadcastedInvokeTxn as OldInvoke, BroadcastedTxn as OldTxn};
    use mp_transactions::{validated::TxTimestamp, IntoStarknetApiExt};
    let OldTxn::Invoke(OldInvoke::V3(mut tx)) =
        crate::test_utils::devnet_transfer_tx(backend, account, Felt::ZERO, false)
    else {
        unreachable!("devnet transfer helper always constructs an invoke v3")
    };
    let call = || Call {
        to: backend.chain_config().native_fee_token_address.to_felt(),
        selector: Selector::from("transfer"),
        calldata: vec![account.address, Felt::ONE, Felt::ZERO],
    };
    tx.calldata = Multicall::default().with(call()).with(call()).flatten().collect::<Vec<_>>().into();
    let hash = OldTxn::Invoke(OldInvoke::V3(tx.clone()))
        .into_validated_tx(
            backend.chain_config().chain_id.to_felt(),
            backend.chain_config().latest_protocol_version,
            TxTimestamp::now(),
        )
        .unwrap()
        .hash;
    let signature = account.secret.sign(&hash).unwrap();
    tx.signature = vec![signature.r, signature.s].into();
    serde_json::from_value(serde_json::to_value(OldTxn::Invoke(OldInvoke::V3(tx))).unwrap()).unwrap()
}

struct RecordingSubmitter {
    attempts: AtomicUsize,
    reject: bool,
    calls: std::sync::Mutex<Vec<(serde_json::Value, Vec<String>)>>,
}

#[async_trait]
impl SubmitTransaction for RecordingSubmitter {
    async fn submit_committed_data_transaction(
        &self,
        transaction: BroadcastedTxn,
        dependencies: Vec<String>,
    ) -> Result<Felt, SubmitTransactionError> {
        self.calls.lock().unwrap().push((serde_json::to_value(&transaction).unwrap(), dependencies));
        match transaction {
            BroadcastedTxn::Invoke(tx) => Ok(self.submit_invoke_transaction(tx).await?.transaction_hash),
            _ => Err(SubmitTransactionError::Unsupported),
        }
    }

    async fn submit_declare_transaction(
        &self,
        _tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(SubmitTransactionError::Unsupported)
    }

    async fn submit_deploy_account_transaction(
        &self,
        _tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(SubmitTransactionError::Unsupported)
    }

    async fn submit_invoke_transaction(
        &self,
        _tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.reject {
            Err(SubmitTransactionError::Unsupported)
        } else {
            Ok(AddInvokeTransactionResult { transaction_hash: Felt::THREE })
        }
    }
}

#[tokio::test]
async fn combined_rpc_imports_before_invoke_and_does_not_submit_after_import_failure() {
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    let (backend, mut starknet, keys) = crate::test_utils::rpc_test_setup_with_execution().await;
    let submitter =
        Arc::new(RecordingSubmitter { attempts: AtomicUsize::new(0), reject: false, calls: Default::default() });
    starknet.transaction_submitter = submitter.clone();
    let account = keys.0.first().unwrap();
    let tx = signed_two_call_tx(&backend, account);
    let values: Vec<_> = vec![(Felt::ZERO, Felt::from(42_u32))];
    let root = CommittedDataSet::new(values.clone()).unwrap().root();
    let funding_values: Vec<_> = vec![(Felt::ZERO, Felt::from(43_u32))];
    let funding_root = CommittedDataSet::new(funding_values.clone()).unwrap().root();

    let call = |funding_root, tx: &BroadcastedTxn| {
        serde_json::json!({"jsonrpc":"2.0","id":1,
            "method":"madara_V0_1_0_importCommittedDataAndSubmitTransaction",
            "params":[[{"root":root,"entries":values,
                        "update":{"application_id":"0x1","dataset_id":"0x2","base_root":null}},
                       {"root":funding_root,"entries":funding_values,
                        "update":{"application_id":"0x1","dataset_id":"0x2","base_root":root}}],tx]})
        .to_string()
    };
    let module = crate::rpc_api_committed_data(&starknet).unwrap();
    let (response, _) = module.raw_json_request(&call(funding_root, &tx), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["error"]["code"], -32003);
    assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().is_none());
    starknet.committed_data_transaction_submission = true;
    let module = crate::rpc_api_committed_data(&starknet).unwrap();
    let (response, _) = module.raw_json_request(&call(funding_root, &tx), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["error"]["code"], -32003);
    assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().is_none());
    starknet.ctx.service_remove(MadaraServiceId::L2Sync);
    let (response, _) = module.raw_json_request(&call(Felt::ONE, &tx), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(response.get("error").is_some(), "invalid second root must fail: {response}");
    assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
    assert!(backend.committed_data_witness(funding_root, Felt::ZERO).unwrap().is_none());
    assert_eq!(submitter.attempts.load(Ordering::SeqCst), 0);

    let (response, _) = module.raw_json_request(&call(funding_root, &tx), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["result"], serde_json::json!({"transaction_hash": Felt::THREE}));
    // No receipt was inserted: admission must return without waiting for a block.
    assert!(backend.view_on_latest().find_transaction_by_hash(&Felt::THREE).unwrap().is_none());
    assert_eq!(submitter.attempts.load(Ordering::SeqCst), 1);
    let update = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
    assert_eq!(
        *submitter.calls.lock().unwrap(),
        vec![(
            serde_json::to_value(&tx).unwrap(),
            vec![
                backend.recovery_reference(Some(update), root).unwrap(),
                backend
                    .recovery_reference(Some(CommittedDataUpdate { base_root: Some(root), ..update }), funding_root)
                    .unwrap(),
            ]
        )]
    );
    assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
    assert!(backend.committed_data_witness(funding_root, Felt::ZERO).unwrap().unwrap().verify());
}

#[tokio::test]
async fn rejected_submission_leaves_only_an_unused_durable_dataset() {
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    let (backend, mut starknet, keys) = crate::test_utils::rpc_test_setup_with_execution().await;
    starknet.ctx.service_remove(MadaraServiceId::L2Sync);
    starknet.committed_data_transaction_submission = true;
    let submitter =
        Arc::new(RecordingSubmitter { attempts: AtomicUsize::new(0), reject: true, calls: Default::default() });
    starknet.transaction_submitter = submitter.clone();
    let tx = signed_two_call_tx(&backend, keys.0.first().unwrap());
    let values: Vec<_> = vec![(Felt::ZERO, Felt::from(99_u32))];
    let root = CommittedDataSet::new(values.clone()).unwrap().root();
    let module = crate::rpc_api_committed_data(&starknet).unwrap();
    let request = serde_json::json!({"jsonrpc":"2.0","id":1,
        "method":"madara_V0_1_0_importCommittedDataAndSubmitTransaction",
        "params":[[{"root":root,"entries":values}],tx]})
    .to_string();
    let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(response.get("error").is_some(), "submission must fail: {response}");
    assert_eq!(submitter.attempts.load(Ordering::SeqCst), 1);
    assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
}

#[tokio::test]
async fn dedicated_module_only_imports_without_admin_or_read_exposure() {
    use crate::{rpc_api_admin, rpc_api_committed_data, rpc_api_user, test_utils::rpc_test_setup};
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    let (_, mut starknet) = rpc_test_setup();
    let module = rpc_api_committed_data(&starknet).unwrap();
    let mut names: Vec<_> = module.method_names().collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "madara_V0_1_0_importCommittedDataAndSubmitTransaction",
            "madara_V0_1_0_importCommittedDataSet",
            "madara_V0_1_0_updateCommittedDataSet"
        ]
    );
    starknet.set_rpc_unsafe_enabled(true);
    for api in [rpc_api_user(&starknet).unwrap(), rpc_api_admin(&starknet).unwrap()] {
        for name in &names {
            let req = serde_json::json!({"jsonrpc":"2.0", "id":1,"method":name,"params":[]}).to_string();
            let (response, _) = api.raw_json_request(&req, 1).await.unwrap();
            let response: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert_eq!(response["error"]["code"], -32601);
        }
    }
    let values: Vec<_> = vec![(Felt::ZERO, Felt::from(2000_u32)), (Felt::ONE, Felt::from(3000_u32))];
    let root = CommittedDataSet::new(values.clone()).unwrap().root();
    let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"madara_V0_1_0_importCommittedDataSet","params":[root,values]}).to_string();
    let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
    let req =
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"madara_V0_1_0_getCommittedDataWitness","params":[root,0]})
            .to_string();
    let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    let req = serde_json::json!({"jsonrpc":"2.0","id":3,"method":"madara_V0_1_0_importCommittedDataSet","params":[Felt::ONE,values]}).to_string();
    let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(response.get("error").is_some(), "Root mismatch must not be acknowledged");
}

#[tokio::test]
async fn committed_data_rpc_rejects_unsigned_before_storage_and_accepts_allowed_signature() {
    let (_, mut starknet) = crate::test_utils::rpc_test_setup();
    let public_key = starknet_types_core::curve::AffinePoint::generator().x();
    starknet.set_committed_data_signers(vec![public_key]).unwrap();
    let module = crate::rpc_api_committed_data(&starknet).unwrap();
    let values: Vec<_> = vec![(Felt::ZERO, Felt::ONE)];
    let root = blockifier::execution::syscalls::committed_data::CommittedDataSet::new(values.clone()).unwrap().root();
    let call = |params| {
        serde_json::json!({"jsonrpc":"2.0","id":1,
        "method":"madara_V0_1_0_importCommittedDataSet","params":params})
        .to_string()
    };
    let (response, _) = module.raw_json_request(&call(serde_json::json!([root, values])), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["error"]["code"], -32001);
    assert!(starknet.backend.committed_data_witness(root, Felt::ZERO).unwrap().is_none());
    let message = import_message_hash(Felt::try_from(&starknet.backend.chain_config().chain_id).unwrap(), root, 1);
    let signature = starknet_core::crypto::ecdsa_sign(&Felt::ONE, &message).unwrap();
    let (response, _) = module
        .raw_json_request(
            &call(serde_json::json!([root, values, [ImportSignature { public_key, r: signature.r, s: signature.s }]])),
            1,
        )
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
    assert!(response.get("error").is_none());
    assert!(starknet.backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
}

#[tokio::test]
async fn committed_data_update_rpc_authenticates_namespace_base_and_expected_root() {
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    // Independent Python signing vector for the initial two-market map.
    let fixture = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
    assert_eq!(
        update_message_hash(
            Felt::from_hex_unchecked("0x534e5f5345504f4c4941"),
            fixture,
            Felt::from_hex_unchecked("0x5a32e2ffd847d637b53eff4f469dcea2ef05175e9b5497282eb9bfbbe14ecda"),
            2
        ),
        Felt::from_hex_unchecked("0x1229be96497ad1baa92f6d7154f9979c23cb11bb31b2620a2715bd650b4ac9d")
    );
    assert!(serde_json::from_value::<CommittedDataUpdate>(serde_json::json!({
        "application_id":"0x1","dataset_id":"0x2"}))
    .is_err());
    let (backend, mut starknet) = crate::test_utils::rpc_test_setup();
    let public_key = starknet_types_core::curve::AffinePoint::generator().x();
    starknet.set_committed_data_signers(vec![public_key]).unwrap();
    let context = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
    let entries = vec![(Felt::MAX, Felt::THREE)];
    let root = CommittedDataSet::new(entries.clone()).unwrap().root();
    let chain = Felt::try_from(&backend.chain_config().chain_id).unwrap();
    let message = update_message_hash(chain, context, root, 1);
    for other in [
        update_message_hash(chain + Felt::ONE, context, root, 1),
        update_message_hash(chain, CommittedDataUpdate { application_id: Felt::TWO, ..context }, root, 1),
        update_message_hash(chain, CommittedDataUpdate { dataset_id: Felt::THREE, ..context }, root, 1),
        update_message_hash(chain, CommittedDataUpdate { base_root: Some(Felt::ZERO), ..context }, root, 1),
        update_message_hash(chain, context, root + Felt::ONE, 1),
        update_message_hash(chain, context, root, 2),
        import_message_hash(chain, root, 1),
    ] {
        assert_ne!(message, other);
    }
    let sig = starknet_core::crypto::ecdsa_sign(&Felt::ONE, &message).unwrap();
    let signature = ImportSignature { public_key, r: sig.r, s: sig.s };
    let module = crate::rpc_api_committed_data(&starknet).unwrap();
    let call = |context: CommittedDataUpdate, signatures: Vec<ImportSignature>| {
        serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"madara_V0_1_0_updateCommittedDataSet",
        "params":[context,root,entries,signatures]})
        .to_string()
    };
    for request in [
        call(context, vec![]),
        call(CommittedDataUpdate { dataset_id: Felt::THREE, ..context }, vec![signature.clone()]),
    ] {
        let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["error"]["code"], -32001);
        assert!(backend.db.get_committed_data_count(root).unwrap().is_none());
    }
    let (response, _) = module.raw_json_request(&call(context, vec![signature]), 1).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(backend.db.list_committed_data_versions(None).unwrap().len(), 1);
}

mod admission;
