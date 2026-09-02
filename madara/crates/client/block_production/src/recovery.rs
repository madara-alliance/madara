//! Startup recovery for persisted preconfirmed execution.
//!
//! The confirmed head remains authoritative. Persisted preconfirmed blocks are
//! re-executed only to reconstruct missing execution artifacts before closing;
//! when persistence is disabled, startup resumes from the confirmed head.

use super::*;

impl BlockProductionTask {
    /// Prepares a PreconfirmedExecutedTransaction for re-execution by converting it to blockifier format.
    ///
    /// This function converts a `PreconfirmedExecutedTransaction` (stored in the database) back into a
    /// blockifier transaction format that can be re-executed. It handles all the necessary conversions
    /// and ensures execution flags are properly set.
    ///
    /// # Process
    ///
    /// 1. Converts `PreconfirmedExecutedTransaction` to `ValidatedTransaction` using `to_validated()`
    /// 2. Sets `charge_fee` based on the `no_charge_fee` configuration (`charge_fee = !no_charge_fee`)
    /// 3. Fetches `declared_class` from state if missing (for Declare transactions)
    /// 4. Converts to blockifier format using `into_blockifier_for_sequencing()` which properly applies execution flags
    ///
    /// # Important Notes
    ///
    /// - The `charge_fee` flag is determined by `self.no_charge_fee` configuration. Note that `new()` is
    ///   called every time Madara starts, so there is no guarantee that the `no_charge_fee` value matches
    ///   the value used during original execution. This is a limitation that should be addressed by storing
    ///   execution configuration in the database (see TODO in `new()`).
    /// - For L1 handler transactions, `paid_fee_on_l1` is preserved from `PreconfirmedExecutedTransaction`
    ///   (stored during `append_batch`) and used during conversion via `to_validated()`
    /// - Declare transactions may need their `declared_class` fetched from state if not already stored
    /// - The conversion uses `into_blockifier_for_sequencing()` which properly sets all execution flags
    ///   including `charge_fee`, `validate`, and `only_query`
    fn prepare_preconfirmed_tx_for_reexecution(
        &self,
        preconfirmed_tx: &PreconfirmedExecutedTransaction,
        state_view: &MadaraStateView,
        no_charge_fee: bool,
    ) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
        // Convert PreconfirmedExecutedTransaction to ValidatedTransaction
        // Use the actual charge_fee value from configuration (charge_fee = !no_charge_fee)
        let mut validated_tx = preconfirmed_tx.to_validated();
        validated_tx.charge_fee = !no_charge_fee;

        // If declared_class is missing and transaction is Declare, fetch it from state_view
        // NOTE: For declare transactions in the preconfirmed block, declared_class MUST be stored
        // during append_batch. If it's None here, that indicates data corruption - we should panic.
        if validated_tx.declared_class.is_none() {
            if let Some(declare_tx) = validated_tx.transaction.as_declare() {
                // This should never happen for declare transactions in the preconfirmed block
                // If it does, it indicates missing data that should have been stored during original execution
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

        // Use into_blockifier_for_sequencing which properly sets execution flags including charge_fee
        let (blockifier_tx, _, _) = validated_tx
            .into_blockifier_for_sequencing()
            .context("Error converting validated transaction to blockifier format for reexecution")?;

        Ok(blockifier_tx)
    }

    /// Helper function to close a preconfirmed block with the given state_diff and bouncer weights.
    /// This is used both during normal block closing (EndBlock case) and during restart recovery.
    /// Returns the result including timing information from the DB layer.
    pub(super) async fn close_preconfirmed_block_with_state_diff(
        backend: Arc<MadaraBackend>,
        block_number: u64,
        bouncer_weights: &blockifier::bouncer::BouncerWeights,
        state_diff: mp_state_update::StateDiff,
    ) -> anyhow::Result<mc_db::AddFullBlockResult> {
        // Copy bouncer_weights to move into the closure (BouncerWeights implements Copy)
        let bouncer_weights = *bouncer_weights;
        global_spawn_rayon_task(move || {
            // Save bouncer weights
            backend
                .write_access()
                .write_bouncer_weights(block_number, &bouncer_weights)
                .context("Saving Bouncer Weights for SNOS")?;

            // Close the preconfirmed block with state_diff
            let result = backend
                .write_access()
                .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, block_number, state_diff)
                .context("Closing preconfirmed block")?;

            anyhow::Ok(result)
        })
        .await
    }

    /// Helper function to get the hash of block_n-10 if it exists.
    fn wait_for_hash_of_block_min_10(
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

    /// Re-executes all transactions in a PreconfirmedBlock to obtain BlockExecutionSummary.
    ///
    /// This function is called when Madara restarts with a preconfirmed block in the database.
    /// It recreates the execution context and re-executes all transactions to regenerate:
    /// - `bouncer_weights`: Resource usage metrics required for block finalization
    /// - `state_diff`: Aggregated state changes needed for block closing
    ///
    /// # Process
    ///
    /// 1. Retrieves all executed transactions from the preconfirmed block
    /// 2. Converts them to blockifier format using `prepare_preconfirmed_tx_for_reexecution()`
    /// 3. Creates `BlockExecutionContext` from the preconfirmed block's header (preserving timestamp, gas_prices, etc.)
    /// 4. Sets up `LayeredStateAdapter` for state access
    /// 5. Creates `TransactionExecutor` with proper `block_n-10` state diff handling (Starknet protocol requirement)
    /// 6. Executes all transactions and calls `finalize()` to get `BlockExecutionSummary`
    ///
    /// # Important Notes
    ///
    /// - The execution context uses the exact header values from the preconfirmed block (timestamp, gas_prices, etc.)
    /// - This ensures re-execution produces the same results as the original execution
    /// - The `block_n-10` state diff entry is set on the `0x1` contract address for protocol compliance
    async fn reexecute_preconfirmed_block(
        &self,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
        saved_chain_config: Option<&Arc<mp_chain_config::ChainConfig>>,
        saved_no_charge_fee: bool,
    ) -> anyhow::Result<BlockExecutionSummary> {
        // Get all executed transactions
        let executed_txs: Vec<_> = preconfirmed_view.borrow_content().executed_transactions().cloned().collect();

        // Get parent block state view
        let parent_state_view = preconfirmed_view.state_view_on_parent();

        // Convert transactions to blockifier format
        // Note: saved_no_charge_fee is passed here to ensure re-execution uses the saved value
        let blockifier_txs: Vec<blockifier::transaction::transaction_execution::Transaction> = executed_txs
            .iter()
            .map(|preconfirmed_tx| {
                self.prepare_preconfirmed_tx_for_reexecution(preconfirmed_tx, &parent_state_view, saved_no_charge_fee)
            })
            .collect::<Result<Vec<_>, _>>()
            .context("Converting preconfirmed transactions to blockifier format")?;

        // Create BlockExecutionContext from PreconfirmedBlock header (preserving exact saved values)
        let header = &preconfirmed_view.block().header;
        let exec_ctx = BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        // Create LayeredStateAdapter
        let state_adapter =
            LayeredStateAdapter::new(self.backend.clone()).context("Creating LayeredStateAdapter for re-execution")?;

        // Create TransactionExecutor with block_n-10 handling
        // Use saved configs if available, otherwise use current backend configs
        let custom_chain_config = saved_chain_config;

        let mut executor = crate::util::create_executor_with_block_n_min_10(
            &self.backend,
            &exec_ctx,
            state_adapter,
            |block_n| Self::wait_for_hash_of_block_min_10(&self.backend, block_n),
            custom_chain_config, // Use saved chain_config if available (re-execution)
        )
        .context("Creating TransactionExecutor for re-execution")?;

        // Execute all transactions
        let execution_results = executor.execute_txs(&blockifier_txs, /* execution_deadline */ None);

        // Verify that re-execution produces matching receipts
        for (i, (result, preconfirmed_tx)) in execution_results.iter().zip(executed_txs.iter()).enumerate() {
            match result {
                Ok((exec_info, _state_maps)) => {
                    // Convert execution info to receipt
                    let reexecuted_receipt = from_blockifier_execution_info(exec_info, &blockifier_txs[i]);

                    // Compare receipts - they should match exactly
                    anyhow::ensure!(
                        reexecuted_receipt.transaction_hash() == preconfirmed_tx.transaction.receipt.transaction_hash(),
                        "Re-execution produced different receipt hash for transaction {} (hash: {:#x})",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );

                    anyhow::ensure!(
                        reexecuted_receipt == preconfirmed_tx.transaction.receipt,
                        "Re-execution produced different receipt content for transaction {} (hash: {:#x})",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );
                }
                Err(err) => {
                    tracing::warn!("Transaction execution error during re-execution: {err:?}");
                    // If execution failed, we can't compare receipts, but this is unexpected
                    anyhow::bail!(
                        "Transaction {} (hash: {:#x}) failed during re-execution: {err:?}",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );
                }
            }
        }

        // Call finalize() to get BlockExecutionSummary
        let block_exec_summary = executor.finalize().context("Finalizing executor to get BlockExecutionSummary")?;

        Ok(block_exec_summary)
    }

    /// Saves current runtime config for future restarts.
    fn save_current_runtime_exec_config(&self) -> anyhow::Result<()> {
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

    /// Closes the last preconfirmed block stored in the database (if any).
    ///
    /// This function is called when Madara restarts and finds a preconfirmed block in the database.
    /// It handles closing the block properly by re-executing transactions to regenerate execution context.
    ///
    /// # Process
    ///
    /// 1. Checks if a preconfirmed block exists.
    /// 2. Re-executes transactions to obtain `bouncer_weights` and `state_diff`.
    /// 3. Extracts L1 handler nonces and cleans up L1-L2 message nonces.
    /// 4. Saves bouncer weights and closes the block.
    /// 5. Updates runtime config for future blocks.
    ///
    /// Note: Re-execution uses saved config values (e.g. `no_charge_fee`) to ensure consistency with original execution.
    /// Runtime config is always saved for persistence.
    pub(super) async fn close_preconfirmed_block_if_exists(&mut self) -> anyhow::Result<()> {
        let head = self.backend.chain_head_state();
        let confirmed_tip = head.confirmed_tip;
        let Some(internal_preconfirmed_tip) = head.internal_preconfirmed_tip else {
            self.save_current_runtime_exec_config()?;
            return Ok(());
        };

        // Startup recovery scans block-keyed preconfirmed entries in ascending order:
        // [confirmed + 1, internal_preconfirmed_tip].
        let start_block_n = confirmed_tip.map(|n| n.saturating_add(1)).unwrap_or(0);
        if start_block_n > internal_preconfirmed_tip {
            self.save_current_runtime_exec_config()?;
            return Ok(());
        }

        if self.discard_preconfirmed_on_startup {
            let preconfirmed_view = self
                .backend
                .block_view_on_preconfirmed(internal_preconfirmed_tip)
                .with_context(|| format!("Getting preconfirmed block view for block #{internal_preconfirmed_tip}"))?;
            let block_number = preconfirmed_view.block_number();
            let n_txs = preconfirmed_view.num_executed_transactions();
            let tx_hashes: Vec<_> = preconfirmed_view
                .get_block_info()
                .tx_hashes
                .into_iter()
                .map(|tx_hash| format!("{tx_hash:#x}"))
                .collect();

            tracing::warn!(
                discarded_transaction_hashes = ?tx_hashes,
                "Discarding preconfirmed block #{} with {} transactions on startup because discard_preconfirmed_on_startup is enabled; these transactions are permanently lost and will not be re-queued",
                block_number,
                n_txs
            );

            let backend = self.backend.clone();
            global_spawn_rayon_task(move || {
                backend.write_access().clear_preconfirmed().context("Discarding preconfirmed block on startup")
            })
            .await?;

            self.save_current_runtime_exec_config()
                .context("Saving runtime execution config after discarding preconfirmed block")?;

            tracing::info!("🧹 Discarded preconfirmed block #{} on startup", block_number);
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

            let block_exec_summary = self
                .reexecute_preconfirmed_block(&preconfirmed_view, saved_chain_config.as_ref(), saved_no_charge_fee)
                .await
                .with_context(|| format!("Re-executing preconfirmed block #{block_number} to get execution summary"))?;

            let old_declared_contracts = preconfirmed_view.get_old_declared_contracts();
            let deployed_contracts_set = preconfirmed_view.get_deployed_contracts_set();
            let migration_v2_hashes: std::collections::HashSet<Felt> = block_exec_summary
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

            let _db_result = Self::close_preconfirmed_block_with_state_diff(
                self.backend.clone(),
                block_number,
                &block_exec_summary.bouncer_weights,
                state_diff,
            )
            .await
            .with_context(|| format!("Closing preconfirmed block #{block_number} on startup"))?;

            tracing::info!("✅ Closed preconfirmed block #{} with {} transactions on startup", block_number, n_txs);
        }

        self.save_current_runtime_exec_config()
            .context("Updating runtime execution config after startup preconfirmed recovery")?;

        Ok(())
    }
}
