use super::state::OptimisticPipelineNotification;
use super::{
    CanonicalizationTaskCanonical, CanonicalizationTaskResult, ComparatorDecision, PendingCanonicalizationInput,
    TaintedRebuildClosePayload, TaintedRebuildSourceTx, TaintedRebuildStepResult, TaskState,
};
use crate::comparator::CanonicalBlockSource;
use crate::executor::{BatchExecutionResult, ExecutorMessage};
use crate::BlockProductionStateNotification;
use crate::{metrics::BlockProductionMetrics, BlockProductionTask, CurrentBlockState};
use blockifier::bouncer::{BouncerConfig, BouncerWeights};
use mc_db::storage::MadaraStorageWrite;
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    MadaraBackend, MadaraBackendConfig, MadaraStorageRead,
};
use mc_devnet::{
    Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector, UDC_CONTRACT_ADDRESS,
};
use mc_mempool::{Mempool, MempoolConfig};
use mc_settlement_client::L1ClientMock;
use mc_submit_tx::{SubmitTransaction, TransactionValidator, TransactionValidatorConfig};
use mp_block::header::PreconfirmedHeader;
use mp_chain_config::ChainConfig;
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
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

type TxFixtureInfo = (Transaction, mp_receipt::TransactionReceipt);

mod block_lifecycle;
mod canonicalization;
mod optimistic_pipeline;
mod publication;
mod startup_recovery;
mod state_tracking;
mod strict_fallback;
mod tainted_rebuild;

use self::strict_fallback::{
    apply_tainted_rebuild_step_result, assert_tx_not_in_mempool, carry_row_from_validated,
    make_preconfirmed_tx_with_hash, make_validated_invoke_tx, persist_preconfirmed_bucket, recv_routed_batch,
    routed_batch_hashes, rust_transfer_routing_cfg, seed_real_preconfirmed_block, spawn_batcher_with_bypass_txs,
    spawn_test_finalizer, tainted_rebuild_control_plane_test_task,
};

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

#[rstest::fixture]
fn bouncer_weights() -> BouncerWeights {
    // The bouncer weights values are configured in such a way
    // that when loaded, the block will close after one transaction
    // is added to it, to test the pending tick closing the block
    BouncerWeights { n_txs: 1, ..BouncerWeights::max() }
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
        )
    }
}

#[rstest::fixture]
pub async fn devnet_setup(
    #[default(Duration::from_secs(30))] block_time: Duration,
    #[default(false)] use_bouncer_weights: bool,
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
            ..ChainConfig::madara_devnet()
        })
    } else {
        Arc::new(ChainConfig { block_time, ..ChainConfig::madara_devnet() })
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

pub async fn tainted_rebuild_spill_devnet_setup() -> DevnetSetup {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let mut genesis = ChainGenesisDescription::base_config().unwrap();
    let contracts = genesis.add_devnet_contracts(10).unwrap();

    let chain_config = Arc::new(ChainConfig {
        block_time: Duration::from_secs(30),
        bouncer_config: BouncerConfig {
            block_max_capacity: BouncerWeights {
                sierra_gas: starknet_api::execution_resources::GasAmount(40_000_000),
                proving_gas: starknet_api::execution_resources::GasAmount(50_000_000),
                ..BouncerWeights::max()
            },
            builtin_weights: Default::default(),
        },
        ..ChainConfig::madara_devnet()
    });

    let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
    backend.set_l1_gas_quote_for_testing();
    genesis.build_and_store(&backend).await.unwrap();

    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let tx_validator = Arc::new(TransactionValidator::new(
        Arc::clone(&mempool) as _,
        Arc::clone(&backend),
        TransactionValidatorConfig::default(),
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
fn make_fake_preconfirmed_tx(nonces: Vec<(Felt, Felt)>) -> mc_db::preconfirmed::PreconfirmedExecutedTransaction {
    use mp_transactions::validated::TxTimestamp;
    mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0::default(),
            )),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt::default()),
        },
        state_diff: mp_state_update::TransactionStateUpdate {
            nonces: nonces.into_iter().collect(),
            ..Default::default()
        },
        declared_class: None,
        arrived_at: TxTimestamp(0),
        paid_fee_on_l1: None,
    }
}

/// Helper: create an empty BlockExecutionSummary for tests.
fn make_empty_block_exec_summary() -> blockifier::blockifier::transaction_executor::BlockExecutionSummary {
    blockifier::blockifier::transaction_executor::BlockExecutionSummary {
        state_diff: Default::default(),
        compressed_state_diff: None,
        bouncer_weights: BouncerWeights::empty(),
        casm_hash_computation_data_sierra_gas: Default::default(),
        casm_hash_computation_data_proving_gas: Default::default(),
        compiled_class_hashes_for_migration: Vec::new(),
        block_info: Default::default(),
    }
}
