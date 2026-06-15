use crate::executor::ExecutorCommandError;
use crate::BlockProductionStateNotification;
use crate::{metrics::BlockProductionMetrics, BlockProductionTask};
use blockifier::bouncer::{BouncerConfig, BouncerWeights};
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    test_utils::l1_handler_tx_with_receipt,
    MadaraBackend,
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
use mp_transactions::compute_hash::calculate_contract_address;
use mp_transactions::IntoStarknetApiExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee, Transaction};
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use starknet_core::utils::get_selector_from_name;
use starknet_types_core::felt::Felt;
use std::{sync::Arc, time::Duration};

type TxFixtureInfo = (Transaction, mp_receipt::TransactionReceipt);

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
            bouncer_config: BouncerConfig {
                block_max_capacity: bouncer_weights,
                builtin_instance_limits: Default::default(),
            },
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

pub fn make_transfer_invoke_tx(
    contract_sender: &DevnetPredeployedContract,
    contract_receiver: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    nonce: Felt,
) -> BroadcastedInvokeTxn {
    let erc20_contract_address =
        Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

    make_invoke_tx(
        contract_sender,
        Multicall::default().with(Call {
            to: erc20_contract_address,
            selector: Selector::from("transfer"),
            calldata: vec![contract_receiver.address, (9_999u128 * 1_000_000_000_000_000_000).into(), Felt::ZERO],
        }),
        backend,
        nonce,
    )
}

/// Replaces the signature with garbage so that the transaction fails the account's
/// `__validate__` at execution time. This is a non-revertible error: the executor rejects the
/// transaction instead of including it as reverted.
pub fn corrupt_invoke_signature(tx: &mut BroadcastedInvokeTxn) {
    let signature = match tx {
        BroadcastedInvokeTxn::V0(tx) => &mut tx.signature,
        BroadcastedInvokeTxn::V1(tx) => &mut tx.signature,
        BroadcastedInvokeTxn::V3(tx) => &mut tx.signature,
        _ => unreachable!("the invoke tx is not query only"),
    };
    *signature = vec![Felt::ONE, Felt::TWO].into();
}

pub async fn sign_and_add_invoke_tx(
    contract_sender: &DevnetPredeployedContract,
    contract_receiver: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    validator: &Arc<TransactionValidator>,
    nonce: Felt,
) {
    let tx = make_transfer_invoke_tx(contract_sender, contract_receiver, backend, nonce);
    validator.submit_invoke_transaction(tx.into()).await.expect("Should accept the transaction");
}

//
// This test verifies that when Madara restarts with a preconfirmed block, `close_preconfirmed_block_if_exists`
// correctly re-executes transactions and produces the same global state root, state diff, and receipts as the original
// execution. This ensures correctness of the restart recovery mechanism.
//
// # Test Process
//
// **Phase 1: Normal Block Production**
// 1. Creates a block with various transaction types (invoke, declare, deploy, L1 handler)
// 2. Closes the block normally and captures:
//    - `global_state_root`
//    - `state_diff`
//    - `header` information
//    - Executed transactions
//
// # Transaction Types Tested
// - **Invoke transactions**: Standard contract calls
// - **Declare transactions**: Class declarations
// - **Deploy transactions**: Contract deployments via UDC
// - **L1 handler transactions**: L1 to L2 messages with `paid_fee_on_l1`
//
// # Key Assertions
//
// - Global state root must match exactly (ensures state consistency)
// - State diff must match (values are the same, order may differ)
// - Header fields must match the preconfirmed block (timestamp, gas_prices, etc.)
// - All transactions must match
// - All receipts must match exactly (ensures execution results are identical)
//
// # Important Notes
//
// - Uses two separate `DevnetSetup` fixtures to ensure clean state isolation
// - State diffs are sorted before comparison to handle ordering differences
// - The test verifies that `paid_fee_on_l1` is preserved for L1 handler transactions
// - The test ensures that re-execution produces deterministic results
#[rstest::rstest]
#[timeout(Duration::from_secs(100))]
#[tokio::test]
async fn test_close_preconfirmed_block_reexecution_matches_normal_closing(
    #[future]
    #[from(devnet_setup)]
    original_devnet_setup: DevnetSetup,
    #[future]
    #[from(devnet_setup)]
    restart_devnet_setup: DevnetSetup,
) {
    // used for phase 1, where we close the block and note down its
    // global_state_root, state_diff, and header info
    let mut original_devnet_setup = original_devnet_setup.await;

    // use for phase 2, where we compare the state of the block after re-execution with the state of the block before re-execution
    let mut restart_devnet_setup = restart_devnet_setup.await;

    // --------------------------------------------------------------
    // | PHASE 1: Close the block and note down its state.          |
    // --------------------------------------------------------------

    // Step 1: Create a block normally with transactions in the original backend
    assert!(original_devnet_setup.mempool.is_empty().await);

    // Helper function to create and execute transactions for testing
    async fn create_and_execute_transactions(setup: &DevnetSetup) -> Felt {
        // 1. Declare a contract
        let declare_res =
            sign_and_add_declare_tx(&setup.contracts.0[0], &setup.backend, &setup.tx_validator, Felt::ZERO).await;

        // 2. Deploy contract through UDC
        let (contract_address, deploy_tx) = make_udc_call(
            &setup.contracts.0[0],
            &setup.backend,
            /* nonce */ Felt::ONE,
            declare_res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        setup.tx_validator.submit_invoke_transaction(deploy_tx.into()).await.unwrap();

        // 3. Invoke transaction
        sign_and_add_invoke_tx(
            &setup.contracts.0[0],
            &setup.contracts.0[1],
            &setup.backend,
            &setup.tx_validator,
            Felt::TWO, // nonce after declare (ZERO) and deploy (ONE)
        )
        .await;

        // 4. Declare transaction (for a different contract)
        sign_and_add_declare_tx(
            &setup.contracts.0[2],
            &setup.backend,
            &setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 5. Another invoke transaction
        sign_and_add_invoke_tx(
            &setup.contracts.0[1],
            &setup.contracts.0[3],
            &setup.backend,
            &setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 6. Add L1 handler transaction
        let paid_fee_on_l1 = 128328u128;
        setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 55, // core contract nonce
                contract_address,
                entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
                calldata: vec![
                    /* from_address */ Felt::THREE,
                    /* arg1 */ Felt::ONE,
                    /* arg2 */ Felt::TWO,
                ]
                .into(),
            },
            paid_fee_on_l1,
        ));

        contract_address
    }

    // Add various transaction types to mempool to test re-execution handles all types correctly
    // All transactions will be in a single block
    let _contract_address = create_and_execute_transactions(&original_devnet_setup).await;

    assert!(!original_devnet_setup.mempool.is_empty().await);

    // Run block production to create and close a block with all transactions
    let mut block_production_task = original_devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();
    let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await.unwrap() });

    // Wait for batch to be executed
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Manually close the block
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
    ctx_clone.cancel_global();
    task.await;

    // Step 2: Capture global_state_root, state_diff, and header info from closed block
    let block_number = original_devnet_setup.backend.latest_confirmed_block_n().unwrap();
    let original_block = original_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let original_block_info = original_block.get_block_info().unwrap();
    let expected_global_state_root = original_block_info.header.global_state_root;
    let expected_state_diff = original_block.get_state_diff().unwrap();
    let executed_transactions = original_block.get_executed_transactions(..).unwrap();

    // --------------------------------------------------------------
    // | PHASE 2: Re-execute the block and note down its state.    |
    // --------------------------------------------------------------
    //
    // We'll add them in the same order using the same helper functions
    // All transactions will be in a single block
    // This ensures they're executed in the same context (clean genesis state)
    assert!(restart_devnet_setup.mempool.is_empty().await);

    // Create the same transactions using the helper function
    let _restart_contract_address = create_and_execute_transactions(&restart_devnet_setup).await;

    assert!(!restart_devnet_setup.mempool.is_empty().await);

    // Step 4: Run block production to execute transactions and add them to preconfirmed block
    // Use a very long block_time to prevent auto-closing, then stop manually after batch execution
    let mut restart_block_production_task = restart_devnet_setup.block_prod_task();
    let mut restart_notifications = restart_block_production_task.subscribe_state_notifications();
    let restart_task = AbortOnDrop::spawn(async move {
        restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
    });

    // Wait for batch to be executed (transactions added to preconfirmed block)
    assert_eq!(restart_notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Fetch preconfirmed block view BEFORE dropping the task to avoid race conditions
    let preconfirmed_view = restart_devnet_setup.backend.block_view_on_preconfirmed().unwrap();
    assert_eq!(preconfirmed_view.num_executed_transactions(), executed_transactions.len());
    let restart_preconfirmed_block = preconfirmed_view.block();

    // Stop the task before it closes the block (drop the AbortOnDrop which will abort the task)
    drop(restart_task);

    // Give it a moment to finish current operations
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify preconfirmed block still exists with transactions and no confirmed blocks yet
    assert!(restart_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(0));

    // adding some delay to see if block_timestamp would differ in the reexecution or not
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 5: Now call close_preconfirmed_block_if_exists to re-execute and close the preconfirmed block
    let mut reexec_block_production_task = restart_devnet_setup.block_prod_task();
    reexec_block_production_task.close_preconfirmed_block_if_exists().await.unwrap();

    // Step 6: Verify results match
    assert!(!restart_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(block_number));

    let reexecuted_block_info =
        restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap().get_block_info().unwrap();

    // Verify the header fields match the pre-execution pre-confirmed block's header
    assert_eq!(restart_preconfirmed_block.header.block_timestamp, reexecuted_block_info.header.block_timestamp);
    assert_eq!(restart_preconfirmed_block.header.protocol_version, reexecuted_block_info.header.protocol_version);
    assert_eq!(restart_preconfirmed_block.header.l1_da_mode, reexecuted_block_info.header.l1_da_mode);
    assert_eq!(restart_preconfirmed_block.header.gas_prices, reexecuted_block_info.header.gas_prices);
    assert_eq!(restart_preconfirmed_block.header.sequencer_address, reexecuted_block_info.header.sequencer_address);
    assert_eq!(restart_preconfirmed_block.header.block_number, reexecuted_block_info.header.block_number);

    let reexecuted_block = restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let reexecuted_block_info = reexecuted_block.get_block_info().unwrap();
    let actual_global_state_root = reexecuted_block_info.header.global_state_root;
    let mut actual_state_diff = reexecuted_block.get_state_diff().unwrap();
    let mut expected_state_diff_sorted = expected_state_diff.clone();

    // Sort both state diffs to normalize ordering before comparison
    actual_state_diff.sort();
    expected_state_diff_sorted.sort();

    // Verify global state root matches
    assert_eq!(
        actual_global_state_root, expected_global_state_root,
        "Global state root should match between normal execution and re-execution"
    );

    // Verify state diff matches (after sorting to ignore ordering differences)
    assert_eq!(
            actual_state_diff, expected_state_diff_sorted,
            "State diff should match between normal execution and re-execution (values are the same, only order may differ)"
        );

    // Verify transactions match
    let reexecuted_transactions = reexecuted_block.get_executed_transactions(..).unwrap();
    assert_eq!(reexecuted_transactions, executed_transactions, "Transactions should match");

    // Verify receipts match - re-execution should produce identical receipts
    assert_eq!(executed_transactions.len(), reexecuted_transactions.len(), "Number of transactions should match");
    for (i, (original_tx, reexecuted_tx)) in
        executed_transactions.iter().zip(reexecuted_transactions.iter()).enumerate()
    {
        assert_eq!(
            original_tx.receipt.transaction_hash(),
            reexecuted_tx.receipt.transaction_hash(),
            "Receipt transaction hash should match for transaction {}",
            i
        );
        assert_eq!(
            original_tx.receipt,
            reexecuted_tx.receipt,
            "Receipt should match exactly for transaction {} (hash: {:#x})",
            i,
            original_tx.receipt.transaction_hash()
        );
    }
}

// This test makes sure that the preconfirmed tick closes the block
// if the bouncer capacity is reached
#[ignore] // FIXME: this test is complicated by the fact validation / actual execution fee may differ a bit. Ignore for now.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
#[allow(clippy::too_many_arguments)]
async fn test_block_prod_bouncer_cap_reached_closes_block(
    #[future]
    // Use a very very long block time (longer than the test timeout).
    #[with(Duration::from_secs(10000000), true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // The transaction itself is meaningless, it's just to check
    // if the task correctly reads it and process it
    assert!(devnet_setup.mempool.is_empty().await);
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[2],
        &devnet_setup.contracts.0[3],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    assert!(!devnet_setup.mempool.is_empty().await);

    let mut block_production_task = devnet_setup.block_prod_task();
    // The BouncerConfig is set up with amounts (100000) that should limit
    // the block size in a way that the pending tick on this task
    // closes the block
    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    tokio::time::sleep(Duration::from_secs(5)).await;

    tracing::debug!("{:?}", devnet_setup.backend.block_view_on_latest().map(|l| l.get_executed_transactions(..)));
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    let closed_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    let closed_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
    let preconfirmed_3 = devnet_setup.backend.block_view_on_preconfirmed().unwrap();
    assert_eq!(preconfirmed_3.block_number(), 3);
    assert_eq!(closed_1.get_executed_transactions(..).unwrap().len(), 1);
    // rolled over to next block.
    assert_eq!(closed_2.get_executed_transactions(..).unwrap().len(), 1);
    // rolled over to next block.
    // last block should not be closed though.
    assert_eq!(preconfirmed_3.get_executed_transactions(..).len(), 1);
    assert!(devnet_setup.mempool.is_empty().await);
}

// This test makes sure that the block time tick correctly
// adds the transaction to the preconfirmed block, closes it
// and creates a new empty preconfirmed block
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
#[allow(clippy::too_many_arguments)]
async fn test_block_prod_on_block_time_tick_closes_block(
    #[future]
    #[with(Duration::from_secs(2), true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // The block should be closed after 3s.
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let view = devnet_setup.backend.block_view_on_last_confirmed().unwrap();

    assert_eq!(view.block_number(), 1);
    assert_eq!(view.get_executed_transactions(..).unwrap(), []);
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_no_empty_blocks_does_not_close_empty_block(
    #[future]
    #[with(Duration::from_millis(200), false, true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    assert!(tokio::time::timeout(Duration::from_millis(500), notifications.recv()).await.is_err());
    assert_eq!(devnet_setup.backend.block_view_on_last_confirmed().unwrap().block_number(), 0);
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_l1_handler_tx(
    #[future]
    #[with(Duration::from_secs(3000000000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;
    let mut block_production_task = devnet_setup.block_prod_task();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Declare the contract class.
    let res = sign_and_add_declare_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        /* nonce */ Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    assert_eq!(
        devnet_setup
            .backend
            .block_view_on_preconfirmed()
            .unwrap()
            .get_executed_transaction(0)
            .unwrap()
            .receipt
            .execution_result(),
        ExecutionResult::Succeeded
    );
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    // Deploy contract through UDC.

    let (contract_address, tx) = make_udc_call(
        &devnet_setup.contracts.0[0],
        &devnet_setup.backend,
        /* nonce */ Felt::ONE,
        res.class_hash,
        /* calldata (pubkey) */ &[Felt::TWO],
    );
    devnet_setup.tx_validator.submit_invoke_transaction(tx.into()).await.unwrap();

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    assert_eq!(
        devnet_setup
            .backend
            .block_view_on_preconfirmed()
            .unwrap()
            .get_executed_transaction(0)
            .unwrap()
            .receipt
            .execution_result(),
        ExecutionResult::Succeeded
    );

    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    // Mock the l1 message, block prod should pick it up.

    devnet_setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
        L1HandlerTransaction {
            version: Felt::ZERO,
            nonce: 55, // core contract nonce
            contract_address,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            calldata: vec![/* from_address */ Felt::THREE, /* arg1 */ Felt::ONE, /* arg2 */ Felt::TWO].into(),
        },
        /* paid_fee_on_l1 */ 128328,
    ));

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    let receipt =
        devnet_setup.backend.block_view_on_preconfirmed().unwrap().get_executed_transaction(0).unwrap().receipt;
    assert_eq!(receipt.execution_result(), ExecutionResult::Succeeded);
    tracing::info!("Events = {:?}", receipt.events());
    assert_eq!(receipt.events().len(), 1);

    assert_eq!(
        receipt.events()[0],
        Event {
            from_address: contract_address,
            keys: vec![get_selector_from_name("CalledFromL1").unwrap()],
            data: vec![/* from_address */ Felt::THREE, /* arg1 */ Felt::ONE, /* arg2 */ Felt::TWO]
        }
    );
}

/// Verifies that re-execution uses the saved `no_charge_fee` value.
///
/// # Flow
/// 1. **Initial**: `no_charge_fee = true`. Exec tx, stop before closing. Saved: `true`.
/// 2. **Restart**: `no_charge_fee = false`.
/// 3. **Re-execution**: Uses saved `true` value. Receipts match.
/// 4. **Post**: Config updates to `false` for next block.
#[rstest::rstest]
#[timeout(Duration::from_secs(100))]
#[tokio::test]
async fn test_reexecution_uses_saved_no_charge_fee_value(
    #[future]
    #[from(devnet_setup)]
    original_devnet_setup: DevnetSetup,
) {
    let original_devnet_setup = original_devnet_setup.await;

    // Phase 1: Initial execution with no_charge_fee = true
    let initial_no_charge_fee = true;
    assert!(original_devnet_setup.mempool.is_empty().await);

    // Create a transaction validator that matches our no_charge_fee setting.
    // This ensures transactions are validated with charge_fee = !no_charge_fee.
    // Without this, transactions would be validated with charge_fee = true (default),
    // causing a mismatch between validation and execution.
    let tx_validator_with_no_fee = Arc::new(TransactionValidator::new(
        Arc::clone(&original_devnet_setup.mempool) as _,
        Arc::clone(&original_devnet_setup.backend),
        TransactionValidatorConfig { disable_validation: false, disable_fee: initial_no_charge_fee },
    ));

    sign_and_add_invoke_tx(
        &original_devnet_setup.contracts.0[0],
        &original_devnet_setup.contracts.0[1],
        &original_devnet_setup.backend,
        &tx_validator_with_no_fee,
        Felt::ZERO,
    )
    .await;

    assert!(!original_devnet_setup.mempool.is_empty().await);

    // Start block production task with no_charge_fee = true.
    // This will execute the transaction and add it to the pre-confirmed block.
    let mut block_production_task = BlockProductionTask::new(
        original_devnet_setup.backend.clone(),
        original_devnet_setup.mempool.clone(),
        original_devnet_setup.metrics.clone(),
        Arc::new(original_devnet_setup.l1_client.clone()),
        initial_no_charge_fee,
        false,
    );

    let mut notifications = block_production_task.subscribe_state_notifications();
    let restart_task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Wait for transaction to be executed and added to pre-confirmed block
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Verify pre-confirmed block exists with our transaction
    assert!(original_devnet_setup.backend.has_preconfirmed_block());
    let preconfirmed_view = original_devnet_setup.backend.block_view_on_preconfirmed().unwrap();
    assert_eq!(preconfirmed_view.num_executed_transactions(), 1);

    // Stop the task before it closes the block.
    // This simulates a node crash/restart scenario where a pre-confirmed block exists.
    drop(restart_task);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Phase 2: Restart with different no_charge_fee value
    // This simulates a configuration change between shutdown and restart.
    let restart_no_charge_fee = false;
    let restart_block_production_task = BlockProductionTask::new(
        original_devnet_setup.backend.clone(), // Same backend = same database
        original_devnet_setup.mempool.clone(),
        original_devnet_setup.metrics.clone(),
        Arc::new(original_devnet_setup.l1_client.clone()),
        restart_no_charge_fee, // Current config: no_charge_fee = false
        false,
    );

    // Start the block production task.
    // This will call setup_initial_state() which calls close_preconfirmed_block_if_exists().
    // During re-execution, it will use saved_no_charge_fee = true (from saved config),
    // NOT restart_no_charge_fee = false (from current config).
    let _restart_task = AbortOnDrop::spawn(async move {
        restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
    });

    // Give time for setup_initial_state to complete and close the pre-confirmed block
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Phase 3: Verify block was closed successfully
    assert!(!original_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(original_devnet_setup.backend.latest_confirmed_block_n(), Some(1));

    // Phase 4: Verify config was updated with CURRENT value after re-execution
    // After re-execution completes, the config is updated to the current value.
    // This ensures that the next block will use the current configuration.
    let updated_config = original_devnet_setup
        .backend
        .get_runtime_exec_config()
        .expect("Should be able to read runtime exec config")
        .expect("Runtime exec config should exist after closing");

    assert_eq!(
        updated_config.no_charge_fee, restart_no_charge_fee,
        "Config should be updated with current value after re-execution completes"
    );
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_discard_preconfirmed_on_startup_replaces_runtime_exec_config(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    let devnet_setup = devnet_setup.await;

    let initial_no_charge_fee = true;
    let chain_config = devnet_setup.backend.chain_config();
    let exec_constants = chain_config.exec_constants_by_protocol_version(chain_config.latest_protocol_version).unwrap();
    let saved_runtime_config =
        RuntimeExecutionConfig::from_current_config(chain_config, exec_constants, initial_no_charge_fee).unwrap();

    devnet_setup.backend.write_access().write_runtime_exec_config(&saved_runtime_config).unwrap();
    devnet_setup
        .backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 1, ..Default::default() },
            vec![PreconfirmedExecutedTransaction {
                transaction: l1_handler_tx_with_receipt(55, Felt::from(0x1234_u64)),
                state_diff: Default::default(),
                declared_class: None,
                arrived_at: Default::default(),
                paid_fee_on_l1: Some(0),
            }],
            [],
        ))
        .unwrap();

    assert!(devnet_setup.backend.has_preconfirmed_block());

    let current_no_charge_fee = false;
    let mut restart_block_production_task = BlockProductionTask::new(
        devnet_setup.backend.clone(),
        devnet_setup.mempool.clone(),
        devnet_setup.metrics.clone(),
        Arc::new(devnet_setup.l1_client.clone()),
        current_no_charge_fee,
        true,
    );

    restart_block_production_task.setup_initial_state().await.unwrap();

    assert!(!devnet_setup.backend.has_preconfirmed_block(), "Preconfirmed block should be discarded on startup");
    assert_eq!(
        devnet_setup.backend.latest_confirmed_block_n(),
        Some(0),
        "Discarding startup recovery should keep the latest confirmed block unchanged"
    );

    let updated_config = devnet_setup
        .backend
        .get_runtime_exec_config()
        .expect("Should be able to read runtime exec config")
        .expect("Runtime exec config should exist after discarding");

    assert_eq!(
        updated_config.no_charge_fee, current_no_charge_fee,
        "Discarding startup recovery should replace the saved runtime config with the current one"
    );
}

// This test verifies that graceful shutdown properly closes any open preconfirmed block
// without requiring re-execution. When shutdown is triggered, the block production service
// should close the preconfirmed block using the executor's existing state.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_graceful_shutdown_closes_preconfirmed_block(
    #[future]
    #[with(Duration::from_secs(100), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Step 1: Set up block production with transactions
    assert!(devnet_setup.mempool.is_empty().await);

    // Add a transaction to the mempool
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert!(!devnet_setup.mempool.is_empty().await);

    // Step 2: Start block production and execute a batch to create a preconfirmed block
    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();

    let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

    // Wait for batch to be executed (transactions added to preconfirmed block)
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Verify preconfirmed block exists with transactions
    assert!(devnet_setup.backend.has_preconfirmed_block());
    let preconfirmed_view = devnet_setup.backend.block_view_on_preconfirmed().unwrap();
    assert_eq!(preconfirmed_view.num_executed_transactions(), 1);

    // Step 3: Trigger graceful shutdown by cancelling ServiceContext
    ctx_clone.cancel_global();

    // Step 4: Wait for EndFinalBlock to be processed (indicated by ClosedBlock notification)
    // During graceful shutdown:
    // - Batcher detects cancellation and exits, closing the send_batch channel
    // - Executor detects channel closure and sends EndFinalBlock message
    // - Main loop processes EndFinalBlock and closes the block (sends ClosedBlock notification)
    assert_eq!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock,
        "Expected ClosedBlock notification after EndFinalBlock was processed during graceful shutdown"
    );

    // Step 5: Wait for shutdown to complete
    // All database writes and chain tip updates complete synchronously within the awaited rayon task,
    // so by the time task.await completes, the state is already updated. No delay needed.
    task.await.unwrap();

    // Step 6: Verify the preconfirmed block is closed and saved to database
    assert!(!devnet_setup.backend.has_preconfirmed_block(), "Preconfirmed block should be closed");

    // Verify block was properly closed (check latest confirmed block number)
    let latest_block_n = devnet_setup.backend.latest_confirmed_block_n();
    assert!(latest_block_n.is_some(), "Block should be closed and saved");
    let block_number = latest_block_n.unwrap();

    // Verify transactions are preserved correctly
    let closed_block = devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let executed_transactions = closed_block.get_executed_transactions(..).unwrap();
    assert_eq!(executed_transactions.len(), 1, "Transaction should be preserved in closed block");

    // Verify mempool is empty (transaction was consumed)
    assert!(devnet_setup.mempool.is_empty().await);
}

// This test verifies that graceful shutdown completes successfully when there is no
// preconfirmed block to close. The shutdown should complete without errors.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_graceful_shutdown_with_no_preconfirmed_block(
    #[future]
    #[with(Duration::from_secs(100), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Step 1: Start block production without adding any transactions
    // This ensures no preconfirmed block is created
    assert!(devnet_setup.mempool.is_empty().await);
    assert!(!devnet_setup.backend.has_preconfirmed_block());

    let block_production_task = devnet_setup.block_prod_task();
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();

    let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

    // Step 2: Give a small delay to ensure block production task is running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 3: Verify no preconfirmed block exists
    assert!(!devnet_setup.backend.has_preconfirmed_block());

    // Step 4: Trigger graceful shutdown immediately
    ctx_clone.cancel_global();

    // Step 5: Wait for shutdown to complete - should complete without errors
    // Since there's no preconfirmed block, shutdown should complete immediately
    // without waiting for EndBlock
    task.await.unwrap();

    // Step 6: Verify shutdown completed successfully
    // No preconfirmed block should exist (still)
    assert!(!devnet_setup.backend.has_preconfirmed_block());
}

// Regression test: when a non-empty block is followed by an empty block,
// the timestamp delta should be ~block_time, not ~2*block_time.
//
// Before the fix, create_execution_context() called SystemTime::now() lazily
// — only after wait_take_tx_batch() returned. For a non-empty block, txs
// arrive quickly so the timestamp is set near block-open time. For an empty
// block, the full block_time elapses before the timestamp is set.
// This made the delta between a non-empty and subsequent empty block ≈ 2*block_time.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_empty_block_timestamp_not_drifted(
    #[future]
    #[with(Duration::from_secs(3))]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Submit a transaction so block 1 is non-empty.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Block 1: non-empty (has our tx), closes after block_time.
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    // Block 2: empty, closes after another block_time.
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let block_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    let block_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();

    let ts_1 = block_1.get_block_info().unwrap().header.block_timestamp.0;
    let ts_2 = block_2.get_block_info().unwrap().header.block_timestamp.0;

    let delta = ts_2.saturating_sub(ts_1);

    // With block_time=3s, the delta should be ~3s.
    // Before the fix it would be ~6s (2 * block_time) because:
    //   - block 1 timestamp set at open (near T0)
    //   - block 2 timestamp set after 3s wait (near T0 + 3s + 3s)
    assert!(
        delta >= 2,
        "Timestamp delta between non-empty and subsequent empty block should be ~3s (block_time), \
             but got {delta}s. Timestamps may have stalled or gone backward."
    );
    assert!(
        delta <= 4,
        "Timestamp delta between non-empty and subsequent empty block should be ~3s (block_time), \
             but got {delta}s. This likely means the timestamp is still being set after the block_time wait."
    );
}

// When no_empty_blocks=true, blocks are produced on-demand. The timestamp
// should reflect wall-clock time when the first tx arrives, not the time
// the previous block closed (which could be arbitrarily long ago).
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_no_empty_blocks_timestamp_uses_wall_clock(
    #[future]
    #[with(Duration::from_secs(30), false, true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Submit a tx to trigger block 1.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    // Wait 3 seconds before submitting the next tx. With no_empty_blocks=true,
    // the executor waits indefinitely. The block timestamp should reflect
    // when the tx arrives (~3s from now), not when block 1 closed (~3s ago).
    tokio::time::sleep(Duration::from_secs(3)).await;

    let wall_clock_before_tx = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let block_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
    let ts_2 = block_2.get_block_info().unwrap().header.block_timestamp.0;

    // The timestamp should be within 1s of wall clock when the tx was submitted,
    // not ~3s behind (which would indicate the stale captured time was used).
    let drift = wall_clock_before_tx.saturating_sub(ts_2);
    assert!(
        drift <= 1,
        "With no_empty_blocks=true, block timestamp should reflect wall-clock time \
             when the first tx arrived, but it was {drift}s behind. \
             This likely means a stale captured block_start_time was used."
    );
}

// The bypass channel (used by admin endpoints and chain bootstrapping) lets transactions skip
// the mempool and validation entirely. This test verifies that a transaction submitted through
// the BlockProductionHandle flows through the batcher's bypass stream, gets executed, and ends
// up in the closed block - all without ever touching the mempool.
#[rstest::rstest]
#[timeout(Duration::from_secs(60))]
#[tokio::test]
async fn test_bypass_tx_skips_mempool_and_lands_in_block(
    #[future]
    #[with(Duration::from_secs(10000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Submit through the handle: this goes through the bypass channel, not the mempool.
    let tx = make_transfer_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    let res = control.submit_invoke_transaction(tx.into()).await.unwrap();

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    // The transaction was executed but never went through the mempool.
    assert!(devnet_setup.mempool.is_empty().await);

    let preconfirmed = devnet_setup.backend.block_view_on_preconfirmed().unwrap();
    assert_eq!(preconfirmed.num_executed_transactions(), 1);
    let executed = preconfirmed.get_executed_transaction(0).unwrap();
    assert_eq!(executed.receipt.transaction_hash(), &res.transaction_hash);
    assert_eq!(executed.receipt.execution_result(), ExecutionResult::Succeeded);

    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let closed = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    let txs = closed.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].receipt.transaction_hash(), &res.transaction_hash);
}

// Force-closing through the handle when no transaction has been executed yet must produce an
// empty block (no_empty_blocks is disabled here), and block production must keep working
// afterwards: a transaction submitted later lands in the next block.
#[rstest::rstest]
#[timeout(Duration::from_secs(60))]
#[tokio::test]
async fn test_force_close_produces_empty_block_and_production_continues(
    #[future]
    #[with(Duration::from_secs(10000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Force close with an empty mempool: an empty block #1 must be produced.
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let block_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    assert_eq!(block_1.get_executed_transactions(..).unwrap(), []);

    // Production continues: a transaction submitted afterwards lands in block #2.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let block_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
    let txs = block_2.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].receipt.execution_result(), ExecutionResult::Succeeded);
    assert_eq!(devnet_setup.backend.latest_confirmed_block_n(), Some(2));
}

// A transaction that fails non-revertibly (here: an invalid signature, caught by the account's
// __validate__ at execution time since the bypass channel skips pre-validation) is rejected by
// the executor. This test verifies, at the BlockProductionTask level, that the rejected
// transaction does not abort block production and is excluded from the closed block, while a
// valid transaction submitted alongside it still goes through.
#[rstest::rstest]
#[timeout(Duration::from_secs(60))]
#[tokio::test]
async fn test_rejected_tx_excluded_from_closed_block(
    #[future]
    #[with(Duration::from_secs(10000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Invalid-signature tx through the bypass channel: it skips mempool validation, so it only
    // fails later, inside the executor (rejection, not revert).
    let mut bad_tx = make_transfer_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    corrupt_invoke_signature(&mut bad_tx);
    let bad_res = control.submit_invoke_transaction(bad_tx.into()).await.unwrap();

    // A valid transaction from another account, through the normal mempool path.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    // Rejected transactions count as executed for batch notifications but are never appended
    // to the preconfirmed block. Both txs may land in one or two batches, so wait until the
    // valid one shows up in the preconfirmed block.
    loop {
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
        if devnet_setup.backend.block_view_on_preconfirmed().is_some_and(|view| view.num_executed_transactions() == 1) {
            break;
        }
    }

    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

    let closed = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    let txs = closed.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 1, "Only the valid transaction should be in the closed block");
    assert_ne!(
        txs[0].receipt.transaction_hash(),
        &bad_res.transaction_hash,
        "The rejected transaction must not be included in the closed block"
    );
    assert_eq!(txs[0].receipt.execution_result(), ExecutionResult::Succeeded);
    assert!(devnet_setup.mempool.is_empty().await);
}

// After graceful shutdown, the handle is disconnected: executor commands and bypass
// transaction submissions must fail with an error instead of hanging forever.
#[rstest::rstest]
#[timeout(Duration::from_secs(60))]
#[tokio::test]
async fn test_handle_commands_fail_after_shutdown(
    #[future]
    #[with(Duration::from_secs(100), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let block_production_task = devnet_setup.block_prod_task();
    let control = block_production_task.handle();
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();
    let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

    // Sanity check: the handle works while the task is running.
    tokio::time::sleep(Duration::from_millis(100)).await;
    control.close_block().await.unwrap();

    ctx_clone.cancel_global();
    task.await.unwrap();

    assert!(
        matches!(control.close_block().await, Err(ExecutorCommandError::ChannelClosed)),
        "close_block should fail with ChannelClosed after shutdown"
    );
    let tx = make_transfer_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    assert!(
        control.submit_invoke_transaction(tx.into()).await.is_err(),
        "bypass tx submission should fail after shutdown"
    );
}
