use crate::BlockProductionStateNotification;
use crate::{metrics::BlockProductionMetrics, BlockProductionTask};
use blockifier::bouncer::{BouncerConfig, BouncerWeights};
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    test_utils::l1_handler_tx_with_receipt,
    MadaraBackend, MadaraBackendConfig,
};
use mc_devnet::{
    Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector, UDC_CONTRACT_ADDRESS,
};
use mc_mempool::{Mempool, MempoolConfig};
use mc_settlement_client::L1ClientMock;
use mc_submit_tx::{SubmitTransaction, TransactionValidator, TransactionValidatorConfig};
use mp_block::header::PreconfirmedHeader;
use mp_chain_config::{ChainConfig, RuntimeExecutionConfig};
use mp_convert::ToFelt;
use mp_receipt::{Event, ExecutionResult};
use mp_rpc::v0_9_0::{
    BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedInvokeTxn, BroadcastedTxn, ClassAndTxnHash, DaMode,
    InvokeTxnV3, ResourceBounds, ResourceBoundsMapping,
};
use mp_state_update::StateDiff;
use mp_transactions::compute_hash::calculate_contract_address;
use mp_transactions::IntoStarknetApiExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee, Transaction};
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use starknet_core::utils::get_selector_from_name;
use starknet_types_core::felt::Felt;
use std::{sync::Arc, time::Duration};

type TxFixtureInfo = (Transaction, mp_receipt::TransactionReceipt);

#[rstest::rstest]
#[case::parallel_below_minimum(true, 9, false)]
#[case::parallel_minimum(true, 10, true)]
#[case::serial_any_capacity(false, 1, true)]
fn queue_invariant_matrix(#[case] parallel: bool, #[case] capacity: usize, #[case] expect_ok: bool) {
    let result = crate::close_pipeline::validate_parallel_queue_invariant(parallel, capacity);
    assert_eq!(result.is_ok(), expect_ok);
    if !expect_ok {
        let msg = format!("{:#}", result.expect_err("must fail"));
        assert!(msg.contains("QueueInvariantViolated"));
    }
}

#[test]
fn preconfirmed_runahead_is_accepted_before_previous_close_completes() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block #0 should succeed");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating preconfirmed block #1 should succeed while #0 is still externally visible");

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, None);
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(1));
}

fn empty_state_diff() -> StateDiff {
    StateDiff {
        storage_diffs: vec![],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    }
}

#[rstest::rstest]
#[case::prune_nothing(vec![(11, empty_state_diff()), (12, empty_state_diff())], 10, vec![11, 12])]
#[case::prune_prefix(vec![(10, empty_state_diff()), (11, empty_state_diff()), (12, empty_state_diff())], 10, vec![11, 12])]
#[case::prune_all(vec![(10, empty_state_diff())], 10, vec![])]
fn boundary_prune_matrix(
    #[case] mut input: Vec<(u64, StateDiff)>,
    #[case] completed_block_n: u64,
    #[case] expected_blocks: Vec<u64>,
) {
    crate::close_pipeline::prune_diffs_since_snapshot(&mut input, completed_block_n);
    let remaining_blocks = input.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert_eq!(remaining_blocks, expected_blocks);
}

#[rstest::rstest]
#[case::from_empty_base(vec![(0, empty_state_diff()), (1, empty_state_diff()), (2, empty_state_diff())], None, 2, 3)]
#[case::from_snapshot_floor(vec![(90, empty_state_diff()), (91, empty_state_diff()), (92, empty_state_diff())], Some(89), 92, 3)]
#[case::skip_pruned_prefix(vec![(90, empty_state_diff()), (91, empty_state_diff()), (92, empty_state_diff())], Some(90), 92, 2)]
fn collect_diffs_for_root_from_base_ok(
    #[case] input: Vec<(u64, StateDiff)>,
    #[case] base_block_n: Option<u64>,
    #[case] target_block_n: u64,
    #[case] expected_len: usize,
) {
    let collected = crate::close_pipeline::collect_diffs_for_root_from_base(&input, base_block_n, target_block_n)
        .expect("diff span should be contiguous");
    assert_eq!(collected.len(), expected_len);
}

#[test]
fn collect_diffs_for_root_from_base_rejects_gap() {
    let input = vec![(90, empty_state_diff()), (92, empty_state_diff())];
    let err = crate::close_pipeline::collect_diffs_for_root_from_base(&input, Some(89), 92).expect_err("gap must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("Missing tracked state diff for block #91"));
}

#[rstest::fixture]
fn backend() -> Arc<MadaraBackend> {
    MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()))
}

#[rstest::fixture]
fn bouncer_weights() -> BouncerWeights {
    // The bouncer weights values are configured in such a way
    // that when loaded, the block will close after one transaction
    // is added to it, to test the pending tick closing the block
    BouncerWeights { sierra_gas: starknet_api::execution_resources::GasAmount(1000000), ..BouncerWeights::max() }
}

pub struct DevnetSetup {
    pub backend: Arc<MadaraBackend>,
    pub metrics: Arc<BlockProductionMetrics>,
    pub mempool: Arc<Mempool>,
    pub tx_validator: Arc<TransactionValidator>,
    pub contracts: DevnetKeys,
    pub l1_client: L1ClientMock,
}

impl DevnetSetup {
    pub fn block_prod_task(&mut self) -> BlockProductionTask {
        BlockProductionTask::new(
            self.backend.clone(),
            self.mempool.clone(),
            self.metrics.clone(),
            Arc::new(self.l1_client.clone()),
            false, /* mempool_paused = false */
            false, /* no_charge_fee = false */
            false, /* discard_preconfirmed_on_startup = false */
        )
    }
}

#[rstest::fixture]
pub async fn devnet_setup(
    #[default(Duration::from_secs(30))] block_time: Duration,
    #[default(false)] use_bouncer_weights: bool,
    #[default(false)] no_empty_blocks: bool,
) -> DevnetSetup {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let mut genesis = ChainGenesisDescription::base_config().unwrap();
    let contracts = genesis.add_devnet_contracts(10).unwrap();

    let chain_config: Arc<ChainConfig> = if use_bouncer_weights {
        let bouncer_weights = bouncer_weights();

        Arc::new(ChainConfig {
            block_time,
            bouncer_config: BouncerConfig { block_max_capacity: bouncer_weights, builtin_weights: Default::default() },
            no_empty_blocks,
            ..ChainConfig::madara_devnet()
        })
    } else {
        Arc::new(ChainConfig { block_time, no_empty_blocks, ..ChainConfig::madara_devnet() })
    };

    let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
    backend.set_l1_gas_quote_for_testing();
    genesis.build_and_store(&backend).await.unwrap();

    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let tx_validator = Arc::new(TransactionValidator::new(
        Arc::clone(&mempool) as _,
        Arc::clone(&backend),
        TransactionValidatorConfig::default(), /* disable_fee = false, disable_validation = false */
    ));

    DevnetSetup {
        backend,
        mempool,
        metrics: Arc::new(BlockProductionMetrics::register()),
        tx_validator,
        contracts,
        l1_client: L1ClientMock::new(),
    }
}

#[rstest::fixture]
pub fn tx_invoke_v0(#[default(Felt::ZERO)] contract_address: Felt) -> TxFixtureInfo {
    (
        mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
            mp_transactions::InvokeTransactionV0 { contract_address, ..Default::default() },
        )),
        mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt::default()),
    )
}

#[rstest::fixture]
pub fn tx_l1_handler(#[default(Felt::ZERO)] contract_address: Felt) -> TxFixtureInfo {
    (
        mp_transactions::Transaction::L1Handler(mp_transactions::L1HandlerTransaction {
            contract_address,
            ..Default::default()
        }),
        mp_receipt::TransactionReceipt::L1Handler(mp_receipt::L1HandlerTransactionReceipt::default()),
    )
}

#[rstest::fixture]
fn tx_declare_v0(#[default(Felt::ZERO)] sender_address: Felt) -> TxFixtureInfo {
    (
        mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V0(
            mp_transactions::DeclareTransactionV0 { sender_address, ..Default::default() },
        )),
        mp_receipt::TransactionReceipt::Declare(mp_receipt::DeclareTransactionReceipt::default()),
    )
}

#[rstest::fixture]
pub fn tx_deploy() -> TxFixtureInfo {
    (
        mp_transactions::Transaction::Deploy(mp_transactions::DeployTransaction::default()),
        mp_receipt::TransactionReceipt::Deploy(mp_receipt::DeployTransactionReceipt::default()),
    )
}

#[rstest::fixture]
pub fn tx_deploy_account() -> TxFixtureInfo {
    (
        mp_transactions::Transaction::DeployAccount(mp_transactions::DeployAccountTransaction::V1(
            mp_transactions::DeployAccountTransactionV1::default(),
        )),
        mp_receipt::TransactionReceipt::DeployAccount(mp_receipt::DeployAccountTransactionReceipt::default()),
    )
}

#[rstest::fixture]
pub fn converted_class_legacy(#[default(Felt::ZERO)] class_hash: Felt) -> mp_class::ConvertedClass {
    mp_class::ConvertedClass::Legacy(mp_class::LegacyConvertedClass {
        class_hash,
        info: mp_class::LegacyClassInfo {
            contract_class: Arc::new(mp_class::CompressedLegacyContractClass {
                program: vec![],
                entry_points_by_type: mp_class::LegacyEntryPointsByType {
                    constructor: vec![],
                    external: vec![],
                    l1_handler: vec![],
                },
                abi: None,
            }),
        },
    })
}

#[rstest::fixture]
pub fn converted_class_sierra(
    #[default(Felt::ZERO)] class_hash: Felt,
    #[default(Felt::ZERO)] compiled_class_hash: Felt,
) -> mp_class::ConvertedClass {
    mp_class::ConvertedClass::Sierra(mp_class::SierraConvertedClass {
        class_hash,
        info: mp_class::SierraClassInfo {
            contract_class: Arc::new(mp_class::FlattenedSierraClass {
                sierra_program: vec![],
                contract_class_version: "".to_string(),
                entry_points_by_type: mp_class::EntryPointsByType {
                    constructor: vec![],
                    external: vec![],
                    l1_handler: vec![],
                },
                abi: "".to_string(),
            }),
            compiled_class_hash: Some(compiled_class_hash),
            compiled_class_hash_v2: None,
        },
        compiled: Arc::new(mp_class::CompiledSierra("".to_string())),
    })
}

pub fn make_declare_tx(
    contract: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    nonce: Felt,
) -> BroadcastedDeclareTxn {
    let sierra_class: starknet_core::types::contract::SierraClass =
        serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class: mp_class::FlattenedSierraClass = sierra_class.clone().flatten().unwrap().into();

    // Use BLAKE hash (v2) for v0.14.1+ compatibility
    let hashes = flattened_class.compile_to_casm_with_hashes().unwrap();
    let compiled_contract_class_hash = hashes.blake_hash;

    let mut declare_txn: BroadcastedDeclareTxn = BroadcastedDeclareTxn::V3(BroadcastedDeclareTxnV3 {
        sender_address: contract.address,
        compiled_class_hash: compiled_contract_class_hash,
        // this field will be filled below
        signature: vec![].into(),
        nonce,
        contract_class: flattened_class.into(),
        resource_bounds: ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
            l2_gas: ResourceBounds { max_amount: 10000000000, max_price_per_unit: 10000000 },
            l1_data_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 60000 },
        },
        tip: 0,
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: DaMode::L1,
        fee_data_availability_mode: DaMode::L1,
    });

    let (api_tx, _class) = BroadcastedTxn::Declare(declare_txn.clone())
        .into_starknet_api(backend.chain_config().chain_id.to_felt(), backend.chain_config().latest_protocol_version)
        .unwrap();
    let signature = contract.secret.sign(&api_tx.tx_hash().0).unwrap();

    let tx_signature = match &mut declare_txn {
        BroadcastedDeclareTxn::V1(tx) => &mut tx.signature,
        BroadcastedDeclareTxn::V2(tx) => &mut tx.signature,
        BroadcastedDeclareTxn::V3(tx) => &mut tx.signature,
        _ => unreachable!("the declare tx is not query only"),
    };
    *tx_signature = vec![signature.r, signature.s].into();
    declare_txn
}

pub async fn sign_and_add_declare_tx(
    contract: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    validator: &Arc<TransactionValidator>,
    nonce: Felt,
) -> ClassAndTxnHash {
    validator
        .submit_declare_transaction(make_declare_tx(contract, backend, nonce))
        .await
        .expect("Should accept the transaction")
}

pub fn make_invoke_tx(
    contract_sender: &DevnetPredeployedContract,
    multicall: Multicall,
    backend: &Arc<MadaraBackend>,
    nonce: Felt,
) -> BroadcastedInvokeTxn {
    let mut invoke_txn: BroadcastedInvokeTxn = BroadcastedInvokeTxn::V3(InvokeTxnV3 {
        sender_address: contract_sender.address,
        calldata: multicall.flatten().collect::<Vec<_>>().into(),
        // this field will be filled below
        signature: vec![].into(),
        nonce,
        resource_bounds: ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
            l2_gas: ResourceBounds { max_amount: 10000000000, max_price_per_unit: 10000000 },
            l1_data_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 60000 },
        },
        tip: 0,
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: DaMode::L1,
        fee_data_availability_mode: DaMode::L1,
    });

    let (api_tx, _classes) = BroadcastedTxn::Invoke(invoke_txn.clone())
        .into_starknet_api(backend.chain_config().chain_id.to_felt(), backend.chain_config().latest_protocol_version)
        .unwrap();
    let signature = contract_sender.secret.sign(&api_tx.tx_hash()).unwrap();

    let tx_signature = match &mut invoke_txn {
        BroadcastedInvokeTxn::V0(tx) => &mut tx.signature,
        BroadcastedInvokeTxn::V1(tx) => &mut tx.signature,
        BroadcastedInvokeTxn::V3(tx) => &mut tx.signature,
        _ => unreachable!("the invoke tx is not query only"),
    };
    *tx_signature = vec![signature.r, signature.s].into();

    invoke_txn
}

pub fn make_udc_call(
    contract_sender: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    nonce: Felt,
    class_hash: Felt,
    constructor_calldata: &[Felt],
) -> (Felt, BroadcastedInvokeTxn) {
    let contract_address = calculate_contract_address(
        /* salt */ Felt::ZERO,
        class_hash,
        constructor_calldata,
        /* deployer_address */ Felt::ZERO,
    );

    (
        contract_address,
        make_invoke_tx(
            contract_sender,
            Multicall::default().with(Call {
                to: UDC_CONTRACT_ADDRESS,
                selector: Selector::from("deployContract"),
                calldata: [
                    class_hash,
                    /* salt */ Felt::ZERO,
                    /* unique */ Felt::ZERO,
                    constructor_calldata.len().into(),
                ]
                .into_iter()
                .chain(constructor_calldata.iter().copied())
                .collect(),
            }),
            backend,
            nonce,
        ),
    )
}

pub async fn sign_and_add_invoke_tx(
    contract_sender: &DevnetPredeployedContract,
    contract_receiver: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    validator: &Arc<TransactionValidator>,
    nonce: Felt,
) {
    let erc20_contract_address =
        Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

    let tx = make_invoke_tx(
        contract_sender,
        Multicall::default().with(Call {
            to: erc20_contract_address,
            selector: Selector::from("transfer"),
            calldata: vec![contract_receiver.address, (9_999u128 * 1_000_000_000_000_000_000).into(), Felt::ZERO],
        }),
        backend,
        nonce,
    );

    validator.submit_invoke_transaction(tx.into()).await.expect("Should accept the transaction");
}

#[path = "tests/execution.rs"]
mod execution;
#[path = "tests/recovery.rs"]
mod recovery;
#[path = "tests/shutdown.rs"]
mod shutdown;
