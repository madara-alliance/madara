use blockifier::blockifier::transaction_executor::TransactionExecutor;
use mc_db::MadaraBackend;
use mc_devnet::{
    Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector, UDC_CONTRACT_ADDRESS,
};
use mc_exec::{LayeredStateAdapter, MadaraBackendExecutionExt};
use mc_external_db::{
    config::ExternalDbConfig, metrics::ExternalDbMetrics, mongodb::MempoolTransactionDocument, test_utils,
    writer::ExternalDbWorker,
};
use mp_block::FullBlockWithoutCommitments;
use mp_chain_config::ChainConfig;
use mp_convert::{Felt, FeltHexDisplay, ToFelt};
use mp_receipt::{
    from_blockifier_execution_info, DeclareTransactionReceipt, DeployAccountTransactionReceipt, ExecutionResult,
    InvokeTransactionReceipt, L1HandlerTransactionReceipt, TransactionReceipt,
};
use mp_rpc::v0_9_0::{
    BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, BroadcastedTxn,
    DaMode, DeployAccountTxnV3, InvokeTxnV3, ResourceBounds, ResourceBoundsMapping,
};
use mp_transactions::{
    compute_hash::calculate_contract_address,
    validated::{TxTimestamp, ValidatedTransaction},
    DeclareTransaction, DeployAccountTransaction, DeployTransaction, IntoStarknetApiExt, InvokeTransaction,
    L1HandlerTransaction, Transaction,
};
use mp_utils::service::ServiceContext;
use starknet_api::block::{BlockInfo, BlockNumber, BlockTimestamp};
use starknet_api::core::ContractAddress;
use starknet_core::types::contract::SierraClass;
use starknet_core::utils::get_selector_from_name;
use starknet_signers::SigningKey;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use testcontainers::{core::IntoContainerPort, runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::OnceCell;

const DEVNET_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

fn make_validated_tx_with_hash(tx_hash: Felt, sender: Felt) -> ValidatedTransaction {
    test_utils::validated_transaction(
        Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(sender, Felt::from_hex_unchecked("0x1")))),
        tx_hash,
        sender,
        None,
    )
}

fn make_block(transactions: Vec<mp_block::TransactionWithReceipt>) -> FullBlockWithoutCommitments {
    FullBlockWithoutCommitments {
        header: Default::default(),
        state_diff: Default::default(),
        transactions,
        events: Default::default(),
    }
}

async fn wait_for_mongo_ready(uri: &str) {
    let mut attempts = 0;
    loop {
        if let Ok(client) = mongodb::Client::with_uri_str(uri).await {
            if client.database("admin").run_command(mongodb::bson::doc! { "ping": 1 }).await.is_ok() {
                break;
            }
        }
        attempts += 1;
        if attempts > 20 {
            panic!("MongoDB did not become ready in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

struct MongoFixture {
    _node: ContainerAsync<GenericImage>,
    uri: String,
}

static MONGO_FIXTURE: OnceCell<MongoFixture> = OnceCell::const_new();

fn resource_bounds() -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: ResourceBounds { max_amount: 60_000, max_price_per_unit: 10_000 },
        l2_gas: ResourceBounds { max_amount: 6_000_000_000, max_price_per_unit: 100_000 },
        l1_data_gas: ResourceBounds { max_amount: 60_000, max_price_per_unit: 10_000 },
    }
}

fn sign_invoke_v3(backend: &Arc<MadaraBackend>, signer: &SigningKey, mut tx: InvokeTxnV3) -> BroadcastedInvokeTxn {
    let (api_tx, _class) = BroadcastedTxn::Invoke(BroadcastedInvokeTxn::V3(tx.clone()))
        .into_starknet_api(backend.chain_config().chain_id.to_felt(), backend.chain_config().latest_protocol_version)
        .unwrap();
    let signature = signer.sign(&api_tx.tx_hash()).unwrap();
    tx.signature = vec![signature.r, signature.s].into();
    BroadcastedInvokeTxn::V3(tx)
}

fn sign_declare_v3(
    backend: &Arc<MadaraBackend>,
    signer: &SigningKey,
    mut tx: BroadcastedDeclareTxnV3,
) -> BroadcastedDeclareTxn {
    let (api_tx, _class) = BroadcastedTxn::Declare(BroadcastedDeclareTxn::V3(tx.clone()))
        .into_starknet_api(backend.chain_config().chain_id.to_felt(), backend.chain_config().latest_protocol_version)
        .unwrap();
    let signature = signer.sign(&api_tx.tx_hash()).unwrap();
    tx.signature = vec![signature.r, signature.s].into();
    BroadcastedDeclareTxn::V3(tx)
}

fn sign_deploy_account_v3(
    backend: &Arc<MadaraBackend>,
    signer: &SigningKey,
    mut tx: DeployAccountTxnV3,
) -> BroadcastedDeployAccountTxn {
    let (api_tx, _class) = BroadcastedTxn::DeployAccount(BroadcastedDeployAccountTxn::V3(tx.clone()))
        .into_starknet_api(backend.chain_config().chain_id.to_felt(), backend.chain_config().latest_protocol_version)
        .unwrap();
    let signature = signer.sign(&api_tx.tx_hash()).unwrap();
    tx.signature = vec![signature.r, signature.s].into();
    BroadcastedDeployAccountTxn::V3(tx)
}

fn make_udc_deploy_call(
    sender: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    nonce: Felt,
    class_hash: Felt,
    constructor_calldata: &[Felt],
) -> (Felt, BroadcastedInvokeTxn) {
    let contract_address = calculate_contract_address(
        Felt::ZERO,
        class_hash,
        constructor_calldata,
        /* deployer_address */ Felt::ZERO,
    );
    let calldata =
        [class_hash, /* salt */ Felt::ZERO, /* unique */ Felt::ZERO, constructor_calldata.len().into()]
            .into_iter()
            .chain(constructor_calldata.iter().copied())
            .collect::<Vec<_>>();
    let invoke = InvokeTxnV3 {
        sender_address: sender.address,
        calldata: Multicall::default()
            .with(Call { to: UDC_CONTRACT_ADDRESS, selector: Selector::from("deployContract"), calldata })
            .flatten()
            .collect::<Vec<_>>()
            .into(),
        signature: vec![].into(),
        nonce,
        resource_bounds: resource_bounds(),
        tip: 0,
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: DaMode::L1,
        fee_data_availability_mode: DaMode::L1,
    };
    (contract_address, sign_invoke_v3(backend, &sender.secret, invoke))
}

fn build_executor(backend: &Arc<MadaraBackend>) -> TransactionExecutor<LayeredStateAdapter> {
    let preconfirmed = backend.block_view_on_preconfirmed_or_fake().unwrap();
    let block_info = preconfirmed.get_block_info();
    let header = block_info.header;
    let block_info = BlockInfo {
        block_number: BlockNumber(header.block_number),
        block_timestamp: BlockTimestamp(header.block_timestamp.0),
        sequencer_address: ContractAddress::try_from(header.sequencer_address).unwrap(),
        gas_prices: (&header.gas_prices).into(),
        use_kzg_da: header.l1_da_mode == mp_chain_config::L1DataAvailabilityMode::Blob,
    };
    let state_adapter = LayeredStateAdapter::new(Arc::clone(backend)).unwrap();
    backend.new_executor_for_block_production(state_adapter, block_info).unwrap()
}

fn execute_validated_transactions(
    backend: &Arc<MadaraBackend>,
    txs: &[ValidatedTransaction],
) -> Vec<Result<TransactionReceipt, String>> {
    let mut executor = build_executor(backend);
    let mut results = Vec::with_capacity(txs.len());
    for tx in txs {
        match tx.clone().into_blockifier_for_sequencing() {
            Ok((blockifier_tx, _ts, _declared_class)) => {
                let mut exec_results =
                    executor.execute_txs(&[blockifier_tx.clone()], /* execution_deadline */ None);
                let exec_result = exec_results.remove(0);
                match exec_result {
                    Ok((exec_info, _state_maps)) => {
                        results.push(Ok(from_blockifier_execution_info(&exec_info, &blockifier_tx)));
                    }
                    Err(err) => results.push(Err(format!("{err:?}"))),
                }
            }
            Err(err) => results.push(Err(format!("{err:?}"))),
        }
    }
    let _ = executor.finalize();
    results
}

async fn mongo_fixture() -> &'static MongoFixture {
    MONGO_FIXTURE
        .get_or_init(|| async {
            let image = GenericImage::new("mongo", "7.0").with_exposed_port(27017.tcp());
            let node = image.start().await.unwrap();
            let port = node.get_host_port_ipv4(27017).await.unwrap();
            let uri = format!("mongodb://127.0.0.1:{port}");
            wait_for_mongo_ready(&uri).await;
            MongoFixture { _node: node, uri }
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_outbox_to_mongo_and_retention() {
    let fixture = mongo_fixture().await;
    let uri = fixture.uri.clone();

    let backend = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
    let metrics = Arc::new(ExternalDbMetrics::register());
    let mut config = ExternalDbConfig::new(uri.clone());
    let unique_suffix = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    config.database_name = format!("madara_test_{unique_suffix}");
    config.batch_size = 10;
    config.flush_interval_ms = 100;
    config.retention_delay_secs = 1;
    config.retention_tick_secs = 1;
    let worker = ExternalDbWorker::new(config.clone(), backend.clone(), "MADARA_TEST".to_string(), metrics);

    let sender = test_utils::devnet_account_address();
    let invoke_hash = Felt::from_hex_unchecked("0xabc");
    let declare_hash = Felt::from_hex_unchecked("0xdef");
    let deploy_account_hash = Felt::from_hex_unchecked("0x111");
    let l1_handler_hash = Felt::from_hex_unchecked("0x222");

    let invoke_tx = make_validated_tx_with_hash(invoke_hash, sender);
    let (declare_tx, declared_class) = test_utils::declare_v3(sender, Felt::from_hex_unchecked("0x2"));
    let declare_tx = ValidatedTransaction {
        transaction: Transaction::Declare(DeclareTransaction::V3(declare_tx)),
        paid_fee_on_l1: None,
        contract_address: sender,
        arrived_at: TxTimestamp::now(),
        declared_class: Some(declared_class),
        hash: declare_hash,
        charge_fee: true,
    };
    let deploy_account_tx = ValidatedTransaction {
        transaction: Transaction::DeployAccount(DeployAccountTransaction::V3(test_utils::deploy_account_v3(
            Felt::from_hex_unchecked("0x3"),
        ))),
        paid_fee_on_l1: None,
        contract_address: sender,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash: deploy_account_hash,
        charge_fee: true,
    };
    let l1_handler_tx = test_utils::validated_l1_handler(l1_handler_hash);

    let txs = vec![invoke_tx, declare_tx, deploy_account_tx, l1_handler_tx];
    for tx in &txs {
        backend.write_external_outbox(tx).unwrap();
    }

    let ctx = ServiceContext::new();
    let handle = tokio::spawn(worker.run(ctx.clone()));

    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let collection =
        client.database(&config.database_name).collection::<MempoolTransactionDocument>(&config.collection_name);
    let tx_ids: Vec<String> = txs.iter().map(|tx| format!("{}", tx.hash.hex_display())).collect();

    let mut attempts = 0;
    loop {
        let mut found = 0;
        for tx_id in &tx_ids {
            if collection.find_one(mongodb::bson::doc! { "_id": tx_id }).await.unwrap().is_some() {
                found += 1;
            }
        }
        if found == tx_ids.len() {
            break;
        }
        attempts += 1;
        if attempts > 30 {
            panic!("Transactions not found in MongoDB in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let remaining: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();
    assert!(remaining.is_empty());

    let block_transactions = vec![
        mp_block::TransactionWithReceipt {
            transaction: txs[0].transaction.clone(),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: invoke_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
        mp_block::TransactionWithReceipt {
            transaction: txs[1].transaction.clone(),
            receipt: TransactionReceipt::Declare(DeclareTransactionReceipt {
                transaction_hash: declare_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
        mp_block::TransactionWithReceipt {
            transaction: txs[2].transaction.clone(),
            receipt: TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                transaction_hash: deploy_account_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
        mp_block::TransactionWithReceipt {
            transaction: txs[3].transaction.clone(),
            receipt: TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
                transaction_hash: l1_handler_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
    ];
    let block = make_block(block_transactions);
    backend.write_access().add_full_block_with_classes(&block, &[], true).unwrap();
    backend.set_latest_l1_confirmed(Some(0)).unwrap();

    let mut attempts = 0;
    loop {
        let mut remaining = 0;
        for tx_id in &tx_ids {
            if collection.find_one(mongodb::bson::doc! { "_id": tx_id }).await.unwrap().is_some() {
                remaining += 1;
            }
        }
        if remaining == 0 {
            break;
        }
        attempts += 1;
        if attempts > 40 {
            panic!("Transactions not deleted by retention in time");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    ctx.cancel_global();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_mongo_roundtrip_execution_matches_direct() {
    let fixture = mongo_fixture().await;
    let uri = fixture.uri.clone();

    let chain_config = Arc::new(ChainConfig::madara_devnet());
    let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
    backend.set_l1_gas_quote_for_testing();
    let mut genesis = ChainGenesisDescription::base_config().unwrap();
    let contracts: DevnetKeys = genesis.add_devnet_contracts(2).unwrap();
    genesis.build_and_store(&backend).await.unwrap();

    let metrics = Arc::new(ExternalDbMetrics::register());
    let mut config = ExternalDbConfig::new(uri.clone());
    let unique_suffix = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    config.database_name = format!("madara_exec_test_{unique_suffix}");
    config.batch_size = 10;
    config.flush_interval_ms = 100;
    let worker = ExternalDbWorker::new(config.clone(), backend.clone(), "MADARA_TEST".to_string(), metrics);

    let sender = &contracts.0[0];
    let chain_id = backend.chain_config().chain_id.to_felt();
    let sn_version = backend.chain_config().latest_protocol_version;

    let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class: mp_class::FlattenedSierraClass = sierra_class.clone().flatten().unwrap().into();
    let hashes = mp_class::FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm_with_hashes().unwrap();
    let compiled_class_hash = hashes.blake_hash;
    let class_hash = sierra_class.class_hash().unwrap();

    let declare_tx = sign_declare_v3(
        &backend,
        &sender.secret,
        BroadcastedDeclareTxnV3 {
            sender_address: sender.address,
            compiled_class_hash,
            signature: vec![].into(),
            nonce: Felt::ZERO,
            contract_class: flattened_class.into(),
            resource_bounds: resource_bounds(),
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        },
    );
    let declare_validated =
        BroadcastedTxn::Declare(declare_tx).into_validated_tx(chain_id, sn_version, TxTimestamp::now()).unwrap();

    let new_account_key = SigningKey::from_random();
    let new_account_pubkey = new_account_key.verifying_key().scalar();
    let new_account_address =
        calculate_contract_address(Felt::ZERO, sender.class_hash, &[new_account_pubkey], Felt::ZERO);

    let transfer_invoke = sign_invoke_v3(
        &backend,
        &sender.secret,
        InvokeTxnV3 {
            sender_address: sender.address,
            calldata: Multicall::default()
                .with(Call {
                    to: DEVNET_STRK_CONTRACT_ADDRESS,
                    selector: Selector::from("transfer"),
                    calldata: vec![new_account_address, Felt::from(1_000u64), Felt::ZERO],
                })
                .flatten()
                .collect::<Vec<_>>()
                .into(),
            signature: vec![].into(),
            nonce: Felt::ONE,
            resource_bounds: resource_bounds(),
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        },
    );
    let transfer_validated =
        BroadcastedTxn::Invoke(transfer_invoke).into_validated_tx(chain_id, sn_version, TxTimestamp::now()).unwrap();

    let deploy_account_tx = sign_deploy_account_v3(
        &backend,
        &new_account_key,
        DeployAccountTxnV3 {
            signature: vec![].into(),
            nonce: Felt::ZERO,
            contract_address_salt: Felt::ZERO,
            constructor_calldata: vec![new_account_pubkey],
            class_hash: sender.class_hash,
            resource_bounds: resource_bounds(),
            tip: 0,
            paymaster_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        },
    );
    let deploy_account_validated = BroadcastedTxn::DeployAccount(deploy_account_tx)
        .into_validated_tx(chain_id, sn_version, TxTimestamp::now())
        .unwrap();

    let (deployed_contract_address, deploy_invoke) =
        make_udc_deploy_call(sender, &backend, Felt::from_hex_unchecked("0x2"), class_hash, &[Felt::TWO]);
    let deploy_invoke_validated =
        BroadcastedTxn::Invoke(deploy_invoke).into_validated_tx(chain_id, sn_version, TxTimestamp::now()).unwrap();

    let l1_handler_tx = ValidatedTransaction {
        transaction: Transaction::L1Handler(L1HandlerTransaction {
            version: Felt::ZERO,
            nonce: 55,
            contract_address: deployed_contract_address,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            calldata: vec![Felt::THREE, Felt::ONE, Felt::TWO].into(),
        }),
        paid_fee_on_l1: Some(123),
        contract_address: deployed_contract_address,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash: Felt::from_hex_unchecked("0xabc123"),
        charge_fee: false,
    };

    let deploy_tx = DeployTransaction {
        version: Felt::ZERO,
        contract_address_salt: Felt::ONE,
        constructor_calldata: vec![Felt::from_hex_unchecked("0x1234")],
        class_hash: backend.view_on_latest().get_contract_class_hash(&UDC_CONTRACT_ADDRESS).unwrap().unwrap(),
    };
    let deploy_tx_hash = deploy_tx.compute_hash(chain_id, /* legacy */ false);
    let deploy_contract_address = deploy_tx.calculate_contract_address();
    let deploy_validated = ValidatedTransaction {
        transaction: Transaction::Deploy(deploy_tx),
        paid_fee_on_l1: None,
        contract_address: deploy_contract_address,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash: deploy_tx_hash,
        charge_fee: true,
    };

    let direct_txs = vec![
        declare_validated.clone(),
        transfer_validated.clone(),
        deploy_account_validated.clone(),
        deploy_invoke_validated.clone(),
        l1_handler_tx.clone(),
        deploy_validated.clone(),
    ];

    for tx in &direct_txs {
        backend.write_external_outbox(tx).unwrap();
    }

    let ctx = ServiceContext::new();
    let handle = tokio::spawn(worker.run(ctx.clone()));

    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let collection =
        client.database(&config.database_name).collection::<MempoolTransactionDocument>(&config.collection_name);
    let tx_ids: Vec<String> = direct_txs.iter().map(|tx| format!("{}", tx.hash.hex_display())).collect();

    let mut attempts = 0;
    loop {
        let mut found = 0;
        for tx_id in &tx_ids {
            if collection.find_one(mongodb::bson::doc! { "_id": tx_id }).await.unwrap().is_some() {
                found += 1;
            }
        }
        if found == tx_ids.len() {
            break;
        }
        attempts += 1;
        if attempts > 40 {
            panic!("Transactions not found in MongoDB in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let mut mongo_txs = Vec::with_capacity(direct_txs.len());
    for tx_id in &tx_ids {
        let doc = collection.find_one(mongodb::bson::doc! { "_id": tx_id }).await.unwrap().unwrap();
        let raw = doc.raw_transaction.bytes;
        let validated: ValidatedTransaction = bincode::deserialize(&raw).unwrap();
        mongo_txs.push(validated);
    }

    let direct_results = execute_validated_transactions(&backend, &direct_txs);
    let mongo_results = execute_validated_transactions(&backend, &mongo_txs);

    assert_eq!(direct_results.len(), mongo_results.len());
    for (idx, (direct, mongo)) in direct_results.iter().zip(mongo_results.iter()).enumerate() {
        match (direct, mongo) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "receipt mismatch at index {idx}"),
            (Err(a), Err(b)) => assert_eq!(a, b, "error mismatch at index {idx}"),
            _ => panic!("mismatched execution outcome at index {idx}: direct={direct:?}, mongo={mongo:?}"),
        }
    }
    if let Err(err) = &direct_results[5] {
        assert!(err.contains("DeployNotSupported"), "expected deploy conversion to be unsupported, got: {err}");
    }

    ctx.cancel_global();
    let _ = handle.await;
}
