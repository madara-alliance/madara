//! Block production service.

use crate::batcher::Batcher;
use crate::metrics::BlockProductionMetrics;
use anyhow::Context;
use executor::{BatchExecutionResult, ExecutorMessage};
use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use mc_db::MadaraBackend;
use mc_exec::execution::TxInfo;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_block::TransactionWithReceipt;
use mp_convert::{Felt, ToFelt};
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::{ClassUpdateItem, DeclaredClassCompiledClass, TransactionStateUpdate};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::TransactionWithHash;
use mp_utils::rayon::global_spawn_rayon_task;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use opentelemetry::KeyValue;
use std::collections::HashSet;
use std::mem;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

mod batcher;
mod executor;
mod handle;
pub mod metrics;
mod util;

pub use handle::BlockProductionHandle;

/// Used for listening to state changes in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProductionStateNotification {
    ClosedBlock,
    BatchExecuted,
}

#[derive(Debug)]
pub(crate) struct CurrentBlockState {
    backend: Arc<MadaraBackend>,
    pub block_number: u64,
    pub consumed_core_contract_nonces: HashSet<u64>,
    /// We need to keep track of deployed contracts, because blockifier can't make the difference between replaced class / deployed contract :/
    pub deployed_contracts: HashSet<Felt>,
}

impl CurrentBlockState {
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64) -> Self {
        Self {
            backend,
            block_number,
            consumed_core_contract_nonces: Default::default(),
            deployed_contracts: Default::default(),
        }
    }
    /// Process the execution result, merging it with the current pending state
    pub async fn append_batch(&mut self, mut batch: BatchExecutionResult) -> anyhow::Result<()> {
        let mut executed = vec![];

        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            if let Some(core_contract_nonce) = blockifier_tx.l1_handler_tx_nonce() {
                // Even when the l1 handler tx is reverted, we mark the nonce as consumed.
                self.consumed_core_contract_nonces
                    .insert(core_contract_nonce.to_felt().try_into().expect("Invalid nonce"));
            }

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                let declared_class = additional_info.declared_class.take().filter(|_| !execution_info.is_reverted());

                let receipt = from_blockifier_execution_info(&execution_info, &blockifier_tx);
                let converted_tx = TransactionWithHash::from(blockifier_tx.clone());

                executed.push(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt { transaction: converted_tx.transaction, receipt },
                    state_diff: TransactionStateUpdate {
                        nonces: state_diff
                            .nonces
                            .into_iter()
                            .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
                            .collect(),
                        contract_class_hashes: state_diff
                            .class_hashes
                            .into_iter()
                            .map(|(contract_addr, class_hash)| {
                                let entry = if !self.deployed_contracts.contains(&contract_addr)
                                    && !self.backend.view_on_latest_confirmed().is_contract_deployed(&contract_addr)?
                                {
                                    self.deployed_contracts.insert(contract_addr.to_felt());
                                    ClassUpdateItem::DeployedContract(class_hash.to_felt())
                                } else {
                                    ClassUpdateItem::ReplacedClass(class_hash.to_felt())
                                };

                                Ok((contract_addr.to_felt(), entry))
                            })
                            .collect::<anyhow::Result<_>>()?,
                        storage_diffs: state_diff
                            .storage
                            .into_iter()
                            .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
                            .collect(),
                        declared_classes: declared_class
                            .iter()
                            .map(|class| {
                                (
                                    *class.class_hash(),
                                    class
                                        .as_sierra()
                                        .map(|class| DeclaredClassCompiledClass::Sierra(class.info.compiled_class_hash))
                                        .unwrap_or(DeclaredClassCompiledClass::Legacy),
                                )
                            })
                            .collect(),
                    },
                    declared_class,
                    arrived_at: additional_info.arrived_at,
                })
            }
        }

        let backend = self.backend.clone();
        global_spawn_rayon_task(move || {
            backend
                .write_access()
                .append_to_preconfirmed(&executed, /* candidates */ [])
                .context("Appending to preconfirmed block")
        })
        .await?;

        let stats = mem::take(&mut batch.stats);
        if stats.n_added_to_block > 0 {
            tracing::info!(
                "🧮 Executed and added {} transaction(s) to the preconfirmed block at height {} - {:.3?}",
                stats.n_added_to_block,
                self.block_number,
                stats.exec_duration,
            );
            tracing::debug!("Tick stats {:?}", stats);
        }
        Ok(())
    }
}

/// Little state machine that helps us following the state transitions the executor thread sends us.
pub(crate) enum TaskState {
    NotExecuting {
        /// [`None`] when the next block to execute is genesis.
        latest_block_n: Option<u64>,
    },
    Executing(CurrentBlockState),
}

/// The block production task consumes transactions from the mempool in batches.
///
/// This is to allow optimistic concurrency. However, the block may get full during batch execution,
/// and we need to re-add the transactions back into the mempool.
///
/// To understand block production in madara, you should probably start with the [`mp_chain_config::ChainConfig`]
/// documentation.
pub struct BlockProductionTask {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    current_state: Option<TaskState>,
    metrics: Arc<BlockProductionMetrics>,
    state_notifications: Option<mpsc::UnboundedSender<BlockProductionStateNotification>>,
    handle: BlockProductionHandle,
    executor_commands_recv: Option<mpsc::UnboundedReceiver<executor::ExecutorCommand>>,
    l1_client: Arc<dyn SettlementClient>,
    bypass_tx_input: Option<mpsc::Receiver<ValidatedTransaction>>,
}

impl BlockProductionTask {
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
        no_charge_fee: bool
    ) -> Self {
        let (sender, recv) = mpsc::unbounded_channel();
        let (bypass_input_sender, bypass_tx_input) = mpsc::channel(16);
        Self {
            backend: backend.clone(),
            mempool,
            current_state: None,
            metrics,
            handle: BlockProductionHandle::new(backend, sender, bypass_input_sender, no_charge_fee),
            state_notifications: None,
            executor_commands_recv: Some(recv),
            l1_client,
            bypass_tx_input: Some(bypass_tx_input),
        }
    }

    pub fn handle(&self) -> BlockProductionHandle {
        self.handle.clone()
    }

    /// This is a channel that helps the testing of the block production task. It is unused outside of tests.
    pub fn subscribe_state_notifications(&mut self) -> mpsc::UnboundedReceiver<BlockProductionStateNotification> {
        let (sender, recv) = mpsc::unbounded_channel();
        self.state_notifications = Some(sender);
        recv
    }

    fn send_state_notification(&mut self, notification: BlockProductionStateNotification) {
        if let Some(sender) = self.state_notifications.as_mut() {
            let _ = sender.send(notification);
        }
    }

    /// Closes the last pending block store in db (if any).
    ///
    /// This avoids re-executing transaction by re-adding them to the [Mempool].
    async fn close_pending_block_if_exists(&mut self) -> anyhow::Result<()> {
        if self.backend.has_preconfirmed_block() {
            tracing::debug!("Close pending block on startup.");
            let backend = self.backend.clone();
            global_spawn_rayon_task(move || {
                backend
                    .write_access()
                    .close_preconfirmed(
                        /* pre_v0_13_2_hash_override */ true, None, /*this won't be none in ideal case*/
                    )
                    .context("Closing preconfirmed block on startup")
            })
            .await?;
        }
        Ok(())
    }

    /// Handles the state machine and its transitions.
    async fn process_reply(&mut self, reply: ExecutorMessage) -> anyhow::Result<()> {
        match reply {
            ExecutorMessage::StartNewBlock { exec_ctx } => {
                tracing::debug!("Received ExecutorMessage::StartNewBlock block_n={}", exec_ctx.block_number);
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::NotExecuting { latest_block_n } = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
                };

                let new_block_n = latest_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
                if new_block_n != exec_ctx.block_number {
                    anyhow::bail!(
                        "Received new block_n={} from executor, expected block_n={}",
                        exec_ctx.block_number,
                        new_block_n
                    )
                }

                let backend = self.backend.clone();
                global_spawn_rayon_task(move || {
                    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(exec_ctx.into_header()))
                })
                .await?;

                self.current_state =
                    Some(TaskState::Executing(CurrentBlockState::new(self.backend.clone(), new_block_n)));
            }
            ExecutorMessage::BatchExecuted(batch_execution_result) => {
                tracing::debug!(
                    "Received ExecutorMessage::BatchExecuted executed_txs={:?}",
                    batch_execution_result.executed_txs
                );
                let current_state = self.current_state.as_mut().context("No current state")?;
                let TaskState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };

                state.append_batch(batch_execution_result).await?;

                self.send_state_notification(BlockProductionStateNotification::BatchExecuted);
            }
            ExecutorMessage::EndBlock(block_exec_summary) => {
                tracing::debug!("Received ExecutorMessage::EndBlock");
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };

                tracing::debug!("Close and save block block_n={}", state.block_number);
                let start_time = Instant::now();

                let n_txs = self
                    .backend
                    .block_view_on_preconfirmed()
                    .context("No current pre-confirmed block")?
                    .num_executed_transactions();

                let backend = self.backend.clone();
                global_spawn_rayon_task(move || {
                    for l1_nonce in state.consumed_core_contract_nonces {
                        // This ensures we remove the nonces for rejected L1 to L2 message transactions. This avoids us from reprocessing them on restart.
                        backend
                            .remove_pending_message_to_l2(l1_nonce)
                            .context("Removing pending message to l2 from database")?;
                    }

                    backend
                        .write_access()
                        .write_bouncer_weights(state.block_number, &block_exec_summary.bouncer_weights)
                        .context("Saving Bouncer Weights for SNOS")?;

                    let state_diff: mp_state_update::StateDiff = block_exec_summary.state_diff.into();
                    backend
                        .write_access()
                        .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, Some(state_diff))
                        .context("Closing block")?;
                    anyhow::Ok(())
                })
                .await?;

                let time_to_close = start_time.elapsed();
                tracing::info!(
                    "⛏️  Closed block #{} with {n_txs} transactions - {time_to_close:?}",
                    state.block_number
                );

                // Record metrics
                let attributes = [
                    KeyValue::new("transactions_added", n_txs.to_string()),
                    KeyValue::new("closing_time", time_to_close.as_secs_f32().to_string()),
                ];

                self.metrics.block_counter.add(1, &[]);
                self.metrics.block_gauge.record(state.block_number, &attributes);
                self.metrics.transaction_counter.add(n_txs as u64, &[]);

                self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(state.block_number) });
                self.send_state_notification(BlockProductionStateNotification::ClosedBlock);
            }
        }

        Ok(())
    }

    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        self.close_pending_block_if_exists().await.context("Cannot close pending block on startup")?;

        // initial state
        let latest_block_n = self.backend.latest_confirmed_block_n();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n });

        Ok(())
    }

    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn run(mut self, ctx: ServiceContext) -> Result<(), anyhow::Error> {
        self.setup_initial_state().await?;

        let mut executor = executor::start_executor_thread(
            Arc::clone(&self.backend),
            self.executor_commands_recv.take().context("Task already started")?,
        )
        .context("Starting executor thread")?;

        // Batcher task is handled in a separate tokio task.
        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let bypass_tx_input = self.bypass_tx_input.take().context("Bypass tx channel already taken")?;
        let mut batcher_task = AbortOnDrop::spawn(
            Batcher::new(
                self.backend.clone(),
                self.mempool.clone(),
                self.l1_client.clone(),
                ctx,
                batch_sender,
                bypass_tx_input,
            )
            .run(),
        );

        // Graceful shutdown: when the service is asked to stop, the `batcher_task` will stop,
        //  which will close the `send_batch` channel (by dropping it). The executor thread then will see that the channel
        //  is closed next time it tries to receive from it. The executor thread shuts down, dropping the `executor.stop` channel,
        //  therefore closing it as well.
        // We will then see the anyhow::Ok(()) result in the stop channel, as per the implementation of [`StopErrorReceiver::recv`].
        // Note that for this to work, we need to make sure the `send_batch` channel is never aliased -
        //  otherwise it will never not be closed automatically.

        loop {
            tokio::select! {

                // Bubble up errors from the batcher task. (tokio JoinHandle)
                res = &mut batcher_task => return res.context("In batcher task"),

                // Process results from the execution
                Some(reply) = executor.replies.recv() => {
                    self.process_reply(reply).await.context("Processing reply from executor thread")?;
                }

                // Bubble up errors from the executor thread, or graceful shutdown.
                // We do this after processing all the replies to ensure we don't lose some of the state by accident.
                res = executor.stop.recv() => return res.context("In executor thread"),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::BlockProductionStateNotification;
    use crate::{metrics::BlockProductionMetrics, BlockProductionTask};
    use blockifier::bouncer::{BouncerConfig, BouncerWeights};
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mc_db::MadaraBackend;
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
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, StorageEntry,
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
                true,
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
                bouncer_config: BouncerConfig {
                    block_max_capacity: bouncer_weights,
                    builtin_weights: Default::default(),
                    blake_weight: Default::default(),
                },
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
                compiled_class_hash,
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

        let (compiled_contract_class_hash, _compiled_class) = flattened_class.compile_to_casm().unwrap();

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
            .into_starknet_api(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
            )
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
            .into_starknet_api(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
            )
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

        validator.submit_invoke_transaction(tx).await.expect("Should accept the transaction");
    }

    /// This test makes sure that if a pending block is already present in db
    /// at startup, then it is closed and stored in db.
    ///
    /// This happens if a full node is shutdown (gracefully or not) midway
    /// during block production.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_pass(
        #[future] devnet_setup: DevnetSetup,
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[from(converted_class_legacy)]
        #[with(Felt::ZERO)]
        converted_class_legacy_0: mp_class::ConvertedClass,
    ) {
        let mut devnet_setup = devnet_setup.await;

        let pending_state_diff = mp_state_update::StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::TWO,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::THREE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
            ],
            old_declared_contracts: vec![Felt::ZERO],
            declared_classes: vec![
                DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::ONE },
                DeclaredClassItem { class_hash: Felt::TWO, compiled_class_hash: Felt::TWO },
            ],
            deployed_contracts: vec![
                DeployedContractItem { address: Felt::TWO, class_hash: Felt::TWO },
                DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE },
            ],
            replaced_classes: vec![],
            nonces: vec![
                NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
                NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
                NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
            ],
        };

        let tx_1 = PreconfirmedExecutedTransaction {
            transaction: mp_block::TransactionWithReceipt { transaction: tx_invoke_v0.0, receipt: tx_invoke_v0.1 },
            state_diff: pending_state_diff.clone().into(),
            declared_class: Some(converted_class_legacy_0.clone()),
            arrived_at: Default::default(),
        };

        devnet_setup
            .backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader { block_number: 1, ..Default::default() },
                [tx_1.clone()],
                [],
            ))
            .unwrap();

        // This should load the pending block from db and close it
        let mut block_production_task = devnet_setup.block_prod_task();
        assert_eq!(devnet_setup.backend.latest_block_n(), Some(1));
        assert_eq!(devnet_setup.backend.latest_confirmed_block_n(), Some(0));
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check this was the case.
        assert_eq!(devnet_setup.backend.latest_block_n(), Some(1));
        assert_eq!(devnet_setup.backend.latest_confirmed_block_n(), Some(1));
        assert!(!devnet_setup.backend.has_preconfirmed_block());

        let block = devnet_setup.backend.block_view_on_latest().unwrap().into_confirmed().unwrap();

        assert_eq!(block.get_executed_transactions(..).unwrap(), vec![tx_1.transaction]);
        assert_eq!(block.get_state_diff().unwrap(), pending_state_diff);
        assert_eq!(
            block.state_view().get_class_info_and_compiled(&Felt::ZERO).unwrap(),
            Some(converted_class_legacy_0)
        );
    }

    // This test makes sure that the pending tick closes the block
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
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

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
    // adds the transaction to the pending block, closes it
    // and creates a new empty pending block
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
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // The block should be closed after 3s.
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

        let view = devnet_setup.backend.block_view_on_last_confirmed().unwrap();

        assert_eq!(view.block_number(), 1);
        assert_eq!(view.get_executed_transactions(..).unwrap(), []);
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
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

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
        devnet_setup.tx_validator.submit_invoke_transaction(tx).await.unwrap();

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
                calldata: vec![
                    /* from_address */ Felt::THREE,
                    /* arg1 */ Felt::ONE,
                    /* arg2 */ Felt::TWO,
                ]
                .into(),
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
}
