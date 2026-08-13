use super::*;

impl BlockProductionTask {
    pub(super) fn prepare_preconfirmed_tx_for_reexecution(
        preconfirmed_tx: &PreconfirmedExecutedTransaction,
        state_view: &MadaraStateView,
        no_charge_fee: bool,
    ) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
        let mut validated_tx = preconfirmed_tx.to_validated();
        validated_tx.charge_fee = !no_charge_fee;

        if validated_tx.declared_class.is_none() {
            if let Some(declare_tx) = validated_tx.transaction.as_declare() {
                validated_tx.declared_class = Some(
                    state_view
                        .get_class_info_and_compiled(declare_tx.class_hash())
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "CRITICAL: Error fetching class for class_hash={:#x} in preconfirmed block. \
                                 This indicates data corruption - declared_class should have been stored during append_batch. Error: {}",
                                declare_tx.class_hash(),
                                e
                            )
                        })?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "CRITICAL: Class not found for class_hash={:#x} in parent state view. \
                                 For declare transactions in the preconfirmed block, declared_class must be stored during append_batch.",
                                declare_tx.class_hash()
                            )
                        })?,
                );
            }
        }

        let (blockifier_tx, _, _) = validated_tx
            .into_blockifier_for_sequencing()
            .context("Error converting validated transaction to blockifier format for reexecution")?;

        Ok(blockifier_tx)
    }

    pub(super) fn prepare_validated_tx_for_reexecution(
        validated_tx: &ValidatedTransaction,
        state_view: &MadaraStateView,
        force_charge_fee: Option<bool>,
    ) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
        let mut validated_tx = validated_tx.clone();
        if let Some(charge_fee) = force_charge_fee {
            validated_tx.charge_fee = charge_fee;
        }

        if validated_tx.declared_class.is_none() {
            if let Some(declare_tx) = validated_tx.transaction.as_declare() {
                validated_tx.declared_class = Some(
                    state_view
                        .get_class_info_and_compiled(declare_tx.class_hash())
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "CRITICAL: Error fetching class for class_hash={:#x} during tainted rebuild reexecution. Error: {}",
                                declare_tx.class_hash(),
                                e
                            )
                        })?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "CRITICAL: Class not found for class_hash={:#x} during tainted rebuild reexecution.",
                                declare_tx.class_hash()
                            )
                        })?,
                );
            }
        }

        let (blockifier_tx, _, _) = validated_tx
            .into_blockifier_for_sequencing()
            .context("Error converting validated transaction to blockifier format for tainted rebuild")?;

        Ok(blockifier_tx)
    }

    pub(super) async fn close_canonical_block_with_state_diff(
        backend: Arc<MadaraBackend>,
        block_number: u64,
        consumed_core_contract_nonces: HashSet<u64>,
        bouncer_weights: &blockifier::bouncer::BouncerWeights,
        state_diff: mp_state_update::StateDiff,
        canonical_header: mp_block::header::PreconfirmedHeader,
        canonical_executed_rows: Vec<PreconfirmedExecutedTransaction>,
    ) -> anyhow::Result<mc_db::AddFullBlockResult> {
        let bouncer_weights = *bouncer_weights;
        global_spawn_rayon_task(move || {
            for l1_nonce in consumed_core_contract_nonces {
                backend
                    .remove_pending_message_to_l2(l1_nonce)
                    .context("Removing pending message to l2 from database")?;
            }

            backend
                .write_access()
                .write_bouncer_weights(block_number, &bouncer_weights)
                .context("Saving Bouncer Weights for SNOS")?;

            let result = backend
                .write_access()
                .close_canonical_block(
                    /* pre_v0_13_2_hash_override */ true,
                    block_number,
                    canonical_header,
                    canonical_executed_rows,
                    state_diff,
                )
                .context("Closing canonical block")?;

            anyhow::Ok(result)
        })
        .await
    }

    pub(super) fn startup_recovery_close_state(
        &self,
        block_number: u64,
        canonical_rows: &[PreconfirmedExecutedTransaction],
        consumed_core_contract_nonces: HashSet<u64>,
    ) -> CurrentBlockState {
        let reverted_count = canonical_rows
            .iter()
            .filter(|tx| {
                matches!(tx.transaction.receipt.execution_result(), mp_receipt::ExecutionResult::Reverted { .. })
            })
            .count();
        let declared_classes_count = canonical_rows.iter().filter(|tx| tx.declared_class.is_some()).count();

        let mut state =
            CurrentBlockState::with_execution_mode(self.backend.clone(), block_number, ExecutionMode::BlockifierOnly);
        state.consumed_core_contract_nonces = consumed_core_contract_nonces;
        state.accumulated_stats.n_batches = 1;
        state.accumulated_stats.n_added_to_block = canonical_rows.len();
        state.accumulated_stats.n_executed = canonical_rows.len();
        state.accumulated_stats.n_reverted = reverted_count;
        state.accumulated_stats.declared_classes = declared_classes_count;
        state
    }

    pub(super) fn wait_for_hash_of_block_min_10(
        backend: &Arc<MadaraBackend>,
        block_n: u64,
    ) -> anyhow::Result<Option<(u64, Felt)>> {
        let Some(block_n_min_10) = block_n.checked_sub(10) else {
            return Ok(None);
        };

        if let Some(view) = backend.block_view_on_confirmed(block_n_min_10) {
            let block_hash = view.get_block_info().context("Getting block hash of block_n - 10")?.block_hash;
            Ok(Some((block_n_min_10, block_hash)))
        } else {
            anyhow::bail!(
                "Cannot fetch block #{block_n_min_10} hash (required for block_n-10 context), block view not found"
            )
        }
    }

    pub(super) fn convert_blockifier_state_maps_for_preconfirmed_row(
        backend: &Arc<MadaraBackend>,
        state_maps: blockifier::state::cached_state::StateMaps,
        deployed_in_block: &mut HashSet<Felt>,
    ) -> anyhow::Result<TransactionStateUpdate> {
        let nonces = state_maps
            .nonces
            .into_iter()
            .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
            .collect();

        let storage_diffs = state_maps
            .storage
            .into_iter()
            .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
            .collect();

        let contract_class_hashes = state_maps
            .class_hashes
            .into_iter()
            .map(|(contract_addr, class_hash)| {
                let addr_felt = contract_addr.to_felt();
                let entry = if !deployed_in_block.contains(&addr_felt)
                    && !backend.view_on_latest_confirmed().is_contract_deployed(&addr_felt)?
                {
                    deployed_in_block.insert(addr_felt);
                    ClassUpdateItem::DeployedContract(class_hash.to_felt())
                } else {
                    ClassUpdateItem::ReplacedClass(class_hash.to_felt())
                };
                Ok((addr_felt, entry))
            })
            .collect::<anyhow::Result<_>>()?;

        Ok(TransactionStateUpdate {
            nonces,
            storage_diffs,
            contract_class_hashes,
            declared_classes: Default::default(),
        })
    }

    pub(super) fn declared_classes_from_original_metadata(
        original_tx: &PreconfirmedExecutedTransaction,
        tx_reverted: bool,
    ) -> anyhow::Result<HashMap<Felt, DeclaredClassCompiledClass>> {
        Self::declared_classes_from_validated_metadata(
            &original_tx.transaction.transaction,
            original_tx.declared_class.as_ref(),
            tx_reverted,
        )
    }

    pub(super) fn declared_classes_from_validated_metadata(
        original_tx: &mp_transactions::Transaction,
        declared_class: Option<&mp_class::ConvertedClass>,
        tx_reverted: bool,
    ) -> anyhow::Result<HashMap<Felt, DeclaredClassCompiledClass>> {
        match (original_tx.as_declare(), declared_class, tx_reverted) {
            (Some(_declare_tx), _, true) => Ok(HashMap::new()),
            (Some(declare_tx), Some(class), false) => {
                anyhow::ensure!(
                    class.class_hash() == declare_tx.class_hash(),
                    "Declare transaction metadata mismatch: tx class_hash={:#x}, stored declared_class class_hash={:#x}",
                    declare_tx.class_hash(),
                    class.class_hash()
                );
                let compiled = class
                    .as_sierra()
                    .and_then(|class| class.info.compiled_class_hash_v2.or(class.info.compiled_class_hash))
                    .map(DeclaredClassCompiledClass::Sierra)
                    .unwrap_or(DeclaredClassCompiledClass::Legacy);
                Ok([(*class.class_hash(), compiled)].into_iter().collect())
            }
            (Some(declare_tx), None, false) => anyhow::bail!(
                "Declare transaction missing declared_class metadata for startup recovery (class_hash={:#x})",
                declare_tx.class_hash()
            ),
            (None, Some(class), _) => anyhow::bail!(
                "Non-declare transaction unexpectedly carried declared_class metadata (class_hash={:#x})",
                class.class_hash()
            ),
            (None, None, _) => Ok(HashMap::new()),
        }
    }

    pub(super) fn validated_tx_from_blockifier(
        tx: blockifier::transaction::transaction_execution::Transaction,
        info: AdditionalTxInfo,
    ) -> ValidatedTransaction {
        let converted_tx = TransactionWithHash::from(tx.clone());
        let paid_fee_on_l1 = match &tx {
            blockifier::transaction::transaction_execution::Transaction::L1Handler(l1_tx) => {
                Some(l1_tx.paid_fee_on_l1.0)
            }
            _ => None,
        };
        let charge_fee = match &tx {
            blockifier::transaction::transaction_execution::Transaction::Account(account_tx) => {
                account_tx.execution_flags.charge_fee
            }
            blockifier::transaction::transaction_execution::Transaction::L1Handler(_) => false,
        };

        ValidatedTransaction {
            transaction: converted_tx.transaction,
            paid_fee_on_l1,
            contract_address: tx.contract_address().to_felt(),
            arrived_at: info.arrived_at,
            declared_class: info.declared_class,
            hash: converted_tx.hash,
            charge_fee,
        }
    }

    pub(super) async fn recover_preconfirmed_block_for_close(
        &self,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
        saved_chain_config: Option<&Arc<mp_chain_config::ChainConfig>>,
        saved_no_charge_fee: bool,
    ) -> anyhow::Result<StartupRecoveredBlockPayload> {
        let executed_txs: Vec<_> = preconfirmed_view.borrow_content().executed_transactions().cloned().collect();
        let header = preconfirmed_view.block().header.clone();
        let consumed_core_contract_nonces = executed_txs
            .iter()
            .filter_map(|tx| tx.transaction.transaction.as_l1_handler().map(|l1_tx| l1_tx.nonce))
            .collect();

        let parent_state_view = preconfirmed_view.state_view_on_parent();

        let blockifier_txs: Vec<blockifier::transaction::transaction_execution::Transaction> = executed_txs
            .iter()
            .map(|preconfirmed_tx| {
                Self::prepare_preconfirmed_tx_for_reexecution(preconfirmed_tx, &parent_state_view, saved_no_charge_fee)
            })
            .collect::<Result<Vec<_>, _>>()
            .context("Converting preconfirmed transactions to blockifier format")?;

        let exec_ctx = crate::util::BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        let state_adapter =
            LayeredStateAdapter::new(self.backend.clone()).context("Creating LayeredStateAdapter for re-execution")?;

        let mut executor = crate::util::create_executor_with_block_n_min_10(
            &self.backend,
            &exec_ctx,
            state_adapter,
            |block_n| Self::wait_for_hash_of_block_min_10(&self.backend, block_n),
            saved_chain_config,
        )
        .context("Creating TransactionExecutor for re-execution")?;

        let execution_results = executor.execute_txs(&blockifier_txs, /* execution_deadline */ None);
        anyhow::ensure!(
            execution_results.len() == executed_txs.len(),
            "Startup recovery for block #{} rebuilt only {} of {} persisted transaction(s)",
            header.block_number,
            execution_results.len(),
            executed_txs.len()
        );

        let mut per_tx_artifacts = Vec::with_capacity(executed_txs.len());
        let mut deployed_in_block = HashSet::new();
        for (i, (result, blockifier_tx)) in execution_results.into_iter().zip(blockifier_txs.iter()).enumerate() {
            match result {
                Ok((execution_info, state_maps)) => {
                    let receipt = from_blockifier_execution_info(&execution_info, blockifier_tx);
                    let tx_state_update = Self::convert_blockifier_state_maps_for_preconfirmed_row(
                        &self.backend,
                        state_maps,
                        &mut deployed_in_block,
                    )
                    .with_context(|| {
                        format!("Converting state maps for recovered tx {} in block #{}", i, header.block_number)
                    })?;
                    per_tx_artifacts.push(ReexecExecutedTxArtifacts { receipt, tx_state_update });
                }
                Err(err) => {
                    anyhow::bail!(
                        "Transaction {} (hash: {:#x}) failed during startup recovery re-execution: {err:?}",
                        i,
                        executed_txs[i].transaction.receipt.transaction_hash()
                    );
                }
            }
        }

        let canonical_rows = Self::build_bre_preconfirmed_rows(&executed_txs, per_tx_artifacts)
            .with_context(|| format!("Building canonical rows for recovered block #{}", header.block_number))?;
        anyhow::ensure!(
            canonical_rows.len() == executed_txs.len(),
            "Startup recovery for block #{} rebuilt only {} canonical row(s) for {} persisted transaction(s)",
            header.block_number,
            canonical_rows.len(),
            executed_txs.len()
        );

        let block_exec_summary = executor.finalize().context("Finalizing executor to get BlockExecutionSummary")?;
        let old_declared_contracts = Self::old_declared_contracts_from_rows(&canonical_rows);
        let deployed_contracts_set = Self::deployed_contracts_set_from_rows(&canonical_rows);
        let migration_v2_hashes: HashSet<Felt> = block_exec_summary
            .compiled_class_hashes_for_migration
            .iter()
            .map(|(v2_hash, _v1_hash)| v2_hash.0)
            .collect();
        let state_diff = mp_state_update::StateDiff::from_blockifier(
            block_exec_summary.state_diff,
            &migration_v2_hashes,
            &deployed_contracts_set,
            old_declared_contracts,
        );

        Ok(StartupRecoveredBlockPayload {
            header,
            canonical_rows,
            state_diff,
            bouncer_weights: block_exec_summary.bouncer_weights,
            consumed_core_contract_nonces,
        })
    }

    pub(super) fn save_current_runtime_exec_config(&self) -> anyhow::Result<()> {
        let current_chain_config = self.backend.chain_config();
        let current_exec_constants = current_chain_config
            .exec_constants_by_protocol_version(current_chain_config.latest_protocol_version)
            .context("Failed to resolve execution constants for latest protocol version")?;

        let runtime_config = RuntimeExecutionConfig::from_current_config(
            current_chain_config,
            current_exec_constants,
            self.no_charge_fee,
        )
        .context("Failed to create runtime execution config")?;

        self.backend
            .write_access()
            .write_runtime_exec_config(&runtime_config)
            .context("Saving runtime execution config")?;

        Ok(())
    }

    pub(super) async fn close_preconfirmed_block_if_exists(&mut self) -> anyhow::Result<()> {
        let head = self.backend.chain_head_state();
        let confirmed_tip = head.confirmed_tip;
        let Some(internal_preconfirmed_tip) = head.internal_preconfirmed_tip else {
            self.save_current_runtime_exec_config()?;
            return Ok(());
        };

        let start_block_n = confirmed_tip.map(|n| n.saturating_add(1)).unwrap_or(0);
        if start_block_n > internal_preconfirmed_tip {
            self.save_current_runtime_exec_config()?;
            return Ok(());
        }

        tracing::debug!(
            "Close preconfirmed blocks on startup from block_n={} to block_n={}",
            start_block_n,
            internal_preconfirmed_tip
        );

        let saved_config = self.backend.get_runtime_exec_config().context("Getting runtime execution config")?;
        let (saved_chain_config, saved_no_charge_fee) = if let Some(config) = saved_config {
            (Some(Arc::new(config.chain_config)), config.no_charge_fee)
        } else {
            tracing::warn!("No saved runtime execution config found, using current configs (backward compatibility)");
            (None, self.no_charge_fee)
        };

        let (close_queue_handle, close_queue_task) = crate::finalizer::FinalizerHandle::spawn(
            self.close_queue_capacity(),
            self.metrics.clone(),
            BlockProductionTask::execute_close_payload,
        );

        let recovery_result: anyhow::Result<()> = async {
            for block_number in start_block_n..=internal_preconfirmed_tip {
                let preconfirmed_view = self
                    .backend
                    .block_view_on_preconfirmed(block_number)
                    .with_context(|| format!("Getting preconfirmed block view for block #{block_number}"))?;

                let n_txs = preconfirmed_view.num_executed_transactions();
                tracing::debug!(
                    "Re-executing {} transaction(s) in preconfirmed block #{} to obtain bouncer_weights and state_diff",
                    n_txs,
                    block_number
                );

                let recovered_block = self
                    .recover_preconfirmed_block_for_close(
                        &preconfirmed_view,
                        saved_chain_config.as_ref(),
                        saved_no_charge_fee,
                    )
                    .await
                    .with_context(|| {
                        format!("Recovering canonical close payload for preconfirmed block #{block_number}")
                    })?;

                let state = self.startup_recovery_close_state(
                    block_number,
                    &recovered_block.canonical_rows,
                    recovered_block.consumed_core_contract_nonces,
                );
                self.enqueue_canonical_close_payload(
                    &close_queue_handle,
                    state,
                    recovered_block.bouncer_weights,
                    recovered_block.state_diff,
                    recovered_block.canonical_rows,
                    recovered_block.header,
                )
                .await
                .with_context(|| format!("Enqueueing startup recovery close payload for block #{block_number}"))?;

                let (expected_block_n, completion_rx) = self
                    .pending_completions
                    .pop_front()
                    .context("Startup recovery close completion missing from pending queue")?;
                let completion = completion_rx
                    .await
                    .context("Startup recovery close queue worker dropped completion channel")?
                    .with_context(|| format!("Executing startup recovery close payload for block #{block_number}"))?;
                self.handle_close_completion(&close_queue_handle, expected_block_n, completion).await.with_context(
                    || format!("Processing startup recovery close completion for block #{block_number}"),
                )?;

                tracing::info!("✅ Closed preconfirmed block #{} with {} transactions on startup", block_number, n_txs);
            }
            Ok(())
        }
        .await;

        drop(close_queue_handle);
        let close_queue_result = close_queue_task.join().await.context("Joining startup recovery close queue");
        recovery_result?;
        close_queue_result?;

        self.save_current_runtime_exec_config()
            .context("Updating runtime execution config after startup preconfirmed recovery")?;

        Ok(())
    }

    pub(super) fn load_saved_runtime_exec_config(
        backend: &Arc<MadaraBackend>,
        fallback_no_charge_fee: bool,
    ) -> anyhow::Result<(Option<Arc<mp_chain_config::ChainConfig>>, bool)> {
        let saved_config = backend.get_runtime_exec_config().context("Getting runtime execution config")?;
        Ok(if let Some(config) = saved_config {
            (Some(Arc::new(config.chain_config)), config.no_charge_fee)
        } else {
            (None, fallback_no_charge_fee)
        })
    }

    pub(super) fn overflow_rebuild_header(
        backend: &Arc<MadaraBackend>,
        block_n: u64,
    ) -> anyhow::Result<PreconfirmedHeader> {
        let (previous_l2_gas_price, previous_l2_gas_used) = backend
            .latest_confirmed_block_n()
            .and_then(|parent_block_n| backend.block_view_on_confirmed(parent_block_n))
            .map(|view| view.get_block_info())
            .transpose()?
            .map(|info| (info.header.gas_prices.strk_l2_gas_price, info.total_l2_gas_used))
            .unwrap_or((0, 0));

        Ok(crate::util::create_execution_context(backend, block_n, previous_l2_gas_price, previous_l2_gas_used)?
            .into_header())
    }

    pub(super) fn confirmed_block_min_10_hash(
        backend: &Arc<MadaraBackend>,
        block_n: u64,
    ) -> anyhow::Result<Option<(u64, Felt)>> {
        let Some(block_n_min_10) = block_n.checked_sub(10) else {
            return Ok(None);
        };
        let Some(view) = backend.block_view_on_confirmed(block_n_min_10) else {
            anyhow::bail!("Missing confirmed block #{} while rebuilding block #{}", block_n_min_10, block_n);
        };
        let block_hash = view.get_block_info().context("Getting confirmed block hash for rebuild")?.block_hash;
        Ok(Some((block_n_min_10, block_hash)))
    }
}
