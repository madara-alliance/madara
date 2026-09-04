use super::*;

/// Handle for writing blocks and advancing the canonical chain head.
///
/// Head-projection operations are serialized by `MadaraBackend`; callers remain responsible for
/// submitting them in canonical order. All methods must run in a Rayon thread-pool context.
pub struct MadaraBackendWriter<D: MadaraStorage> {
    pub(super) inner: Arc<MadaraBackend<D>>,
}

impl<D: MadaraStorage> MadaraBackendWriter<D> {
    /// Advances the canonical confirmed tip without losing newer runtime preconfirmed blocks.
    fn transition_to_confirmed_or_empty(&self, new_confirmed_tip: Option<u64>) -> Result<()> {
        let _projection_guard = self.inner.head_projection_write_lock.lock().expect("Poisoned head projection lock");
        let current_head_state = *self.inner.chain_head_state.borrow();

        let current_preconfirmed_runtime = self.inner.preconfirmed_block_runtime.read().expect("Poisoned lock").clone();
        MadaraBackend::<D>::ensure_runtime_preconfirmed_alignment(current_head_state, &current_preconfirmed_runtime)?;

        let next_chain_head_state = match new_confirmed_tip {
            Some(block_n) => current_head_state.next_for_confirmed(block_n)?,
            None => bail!("Cannot replace chain head to empty"),
        };

        if self.inner.config.save_preconfirmed {
            let new_tip_in_db = if let Some(external_tip) = next_chain_head_state.external_preconfirmed_tip {
                if let Some(block) = runtime_preconfirmed_block(&current_preconfirmed_runtime, external_tip) {
                    storage_tip_from_preconfirmed_block(&block)
                } else {
                    let (header, content) =
                        self.inner.db.get_preconfirmed_block_data(external_tip)?.with_context(|| {
                            format!("Expected persisted preconfirmed block data for block #{external_tip}")
                        })?;
                    StorageHeadProjection::Preconfirmed { header, content }
                }
            } else {
                storage_tip_from_confirmed_or_empty(next_chain_head_state.confirmed_tip)
            };

            if self.inner.db.get_head_projection()? != new_tip_in_db {
                self.inner.db.replace_head_projection(&new_tip_in_db)?;
            }
        } else {
            let new_tip_in_db = storage_tip_from_confirmed_or_empty(next_chain_head_state.confirmed_tip);
            if self.inner.db.get_head_projection()? != new_tip_in_db {
                self.inner.db.replace_head_projection(&new_tip_in_db)?;
            }
        }

        self.inner.publish_head_projection(next_chain_head_state, None)
    }

    /// Adds a preconfirmed block using one atomic read-modify-publish projection transition.
    fn transition_to_preconfirmed(&self, preconfirmed: Arc<PreconfirmedBlock>) -> Result<()> {
        let _projection_guard = self.inner.head_projection_write_lock.lock().expect("Poisoned head projection lock");
        let current_head_state = *self.inner.chain_head_state.borrow();

        let current_preconfirmed_runtime = self.inner.preconfirmed_block_runtime.read().expect("Poisoned lock").clone();
        MadaraBackend::<D>::ensure_runtime_preconfirmed_alignment(current_head_state, &current_preconfirmed_runtime)?;

        let next_chain_head_state = current_head_state.next_for_preconfirmed(preconfirmed.header.block_number)?;

        if self.inner.config.save_preconfirmed {
            let internal_only_advance = current_head_state.external_preconfirmed_tip.is_some()
                && next_chain_head_state.external_preconfirmed_tip == current_head_state.external_preconfirmed_tip
                && next_chain_head_state.internal_preconfirmed_tip != current_head_state.internal_preconfirmed_tip;

            if internal_only_advance {
                self.inner.db.write_preconfirmed_header(&preconfirmed.header)?;
                let executed_transactions: Vec<PreconfirmedExecutedTransaction> =
                    preconfirmed.content.borrow().executed_transactions().cloned().collect();
                if !executed_transactions.is_empty() {
                    self.inner.db.append_preconfirmed_content(
                        preconfirmed.header.block_number,
                        0,
                        &executed_transactions,
                    )?;
                }
            } else {
                self.inner.db.replace_head_projection(&storage_tip_from_preconfirmed_block(preconfirmed.as_ref()))?;
            }
        } else {
            let new_tip_in_db = storage_tip_from_confirmed_or_empty(next_chain_head_state.confirmed_tip);
            if self.inner.db.get_head_projection()? != new_tip_in_db {
                self.inner.db.replace_head_projection(&new_tip_in_db)?;
            }
        }

        self.inner.publish_head_projection(next_chain_head_state, Some(preconfirmed))
    }

    /// Appends transactions to the specified runtime preconfirmed block.
    ///
    /// Addressing the block explicitly prevents a delayed executor batch from being appended to a
    /// different tip. Returns an error when `block_n` is no longer present, including after it has
    /// already been confirmed. Candidate transactions are replaced with `replace_candidates`.
    pub fn append_to_preconfirmed(
        &self,
        block_n: u64,
        executed: &[PreconfirmedExecutedTransaction],
        replace_candidates: impl IntoIterator<Item = Arc<ValidatedTransaction>>,
    ) -> Result<()> {
        let _projection_guard = self.inner.head_projection_write_lock.lock().expect("Poisoned head projection lock");
        let block =
            runtime_preconfirmed_block(&self.inner.preconfirmed_block_runtime.read().expect("Poisoned lock"), block_n)
                .with_context(|| format!("There is no preconfirmed block #{block_n}"))?;

        if self.inner.config.save_preconfirmed {
            let start_tx_index = block.content.borrow().n_executed();
            // We don't save candidate transactions.
            self.inner.db.append_preconfirmed_content(block.header.block_number, start_tx_index as u64, executed)?;
        }

        block.append(executed.iter().cloned(), replace_candidates);

        Ok(())
    }

    /// Returns an error if there is no preconfirmed block. Returns the block hash for the closed block.
    ///
    /// When `state_diff` is provided, this function uses an optimized path that skips the expensive
    /// `get_normalized_state_diff()` computation (which queries the DB for every storage entry).
    /// The provided `state_diff` should already contain all necessary fields including
    /// `old_declared_contracts`, `deployed_contracts`, and `replaced_classes`.
    pub fn close_preconfirmed(
        &self,
        pre_v0_13_2_hash_override: bool,
        block_n: u64,
        state_diff: StateDiff,
    ) -> Result<AddFullBlockResult> {
        let fetch_start = Instant::now();
        let preconfirmed_view = self
            .inner
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("There is no preconfirmed block #{block_n}"))?;
        let (mut block, classes) = preconfirmed_view.get_full_block_without_state_diff()?;
        let fetch_duration = fetch_start.elapsed();
        let fetch_secs = fetch_duration.as_secs_f64();
        metrics().get_full_block_without_state_diff_duration.record(fetch_secs, &[]);
        metrics().get_full_block_without_state_diff_last.record(fetch_secs, &[]);

        block.state_diff = state_diff;

        // Write the block & apply to global trie

        let result = self.write_new_confirmed_inner(&block, &classes, pre_v0_13_2_hash_override, fetch_duration)?;

        self.new_confirmed_block(block.header.block_number)?;

        Ok(result)
    }

    /// Clears the current preconfirmed block. Does nothing when the backend has no preconfirmed block.
    pub fn clear_preconfirmed(&self) -> Result<()> {
        let _projection_guard = self.inner.head_projection_write_lock.lock().expect("Poisoned head projection lock");
        let current_head_state = *self.inner.chain_head_state.borrow();
        let current_preconfirmed_runtime = self.inner.preconfirmed_block_runtime.read().expect("Poisoned lock").clone();
        MadaraBackend::<D>::ensure_runtime_preconfirmed_alignment(current_head_state, &current_preconfirmed_runtime)?;

        let Some(internal_preconfirmed_tip) = current_head_state.internal_preconfirmed_tip else {
            return Ok(());
        };
        let confirmed_tip = current_head_state.confirmed_tip;
        let next_chain_head_state =
            ChainHeadState { confirmed_tip, external_preconfirmed_tip: None, internal_preconfirmed_tip: None };
        next_chain_head_state.validate_cross_field_invariants()?;

        if self.inner.config.save_preconfirmed {
            // Explicit discard removes every persisted runahead row before dropping the external
            // projection. If the process stops between these writes, restart can still rebuild
            // the projected header as an empty preconfirmed block, which is consistent with the
            // caller's request to discard its transactions.
            self.inner.db.delete_preconfirmed_rows_up_to(internal_preconfirmed_tip)?;
        }
        let new_tip_in_db = storage_tip_from_confirmed_or_empty(confirmed_tip);
        if self.inner.db.get_head_projection()? != new_tip_in_db {
            self.inner.db.replace_head_projection(&new_tip_in_db)?;
        }

        self.inner.publish_head_projection(next_chain_head_state, None)
    }

    /// Write the runtime execution configuration to the database.
    pub fn write_runtime_exec_config(&self, config: &mp_chain_config::RuntimeExecutionConfig) -> Result<()> {
        self.inner.db.write_runtime_exec_config(config)
    }

    /// Start a new preconfirmed block on top of the latest confirmed block. Deletes and replaces the current preconfirmed block if present.
    /// Warning: Caller is responsible for ensuring the block_number is the one following the current confirmed block.
    pub fn new_preconfirmed(&self, block: PreconfirmedBlock) -> Result<()> {
        self.transition_to_preconfirmed(Arc::new(block))
    }

    /// Add a block. Returns the block hash.
    /// Warning: Caller is responsible for ensuring the block_number is the one following the current confirmed block.
    pub fn add_full_block_with_classes(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
    ) -> Result<AddFullBlockResult> {
        let block_n = block.header.block_number;
        // For add_full_block_with_classes, no get_full_block_with_classes is needed as block is already provided
        let result = self.write_new_confirmed_inner(block, classes, pre_v0_13_2_hash_override, Duration::ZERO)?;

        self.new_confirmed_block(block_n)?;
        Ok(result)
    }

    /// Loads the parent hash used when turning a preconfirmed header into a confirmed header.
    ///
    /// The empty database uses the protocol-defined zero hash for the genesis parent.
    fn parent_block_hash(&self) -> Result<Felt> {
        self.inner
            .block_view_on_last_confirmed()
            .map(|block| block.get_block_info().map(|info| info.block_hash))
            .transpose()
            .map(|hash| hash.unwrap_or(Felt::ZERO))
    }

    /// Computes block commitments and records the close-path timing for that phase.
    ///
    /// Commitment inputs are identical for inline and precomputed Merkle close paths.
    fn compute_block_commitments(
        &self,
        block: &FullBlockWithoutCommitments,
        timings: &mut CloseBlockTimings,
    ) -> BlockCommitments {
        let started_at = Instant::now();
        let commitments = BlockCommitments::compute(
            &CommitmentComputationContext {
                protocol_version: self.inner.chain_config.latest_protocol_version,
                chain_id: self.inner.chain_config.chain_id.to_felt(),
            },
            &block.transactions,
            &block.state_diff,
            &block.events,
        );
        timings.block_commitments_compute = started_at.elapsed();
        let elapsed = timings.block_commitments_compute.as_secs_f64();
        metrics().block_commitments_compute_duration.record(elapsed, &[]);
        metrics().block_commitments_compute_last.record(elapsed, &[]);
        commitments
    }

    /// Verifies a replay-supplied header against the computed block hash before persistence.
    ///
    /// Taking the staged header preserves one-shot replay semantics on both close paths.
    fn verify_custom_header(
        &self,
        block: &FullBlockWithoutCommitments,
        commitments: &BlockCommitments,
        parent_block_hash: Felt,
        global_state_root: Felt,
        block_hash: Felt,
        replay_path: &'static str,
    ) -> Result<()> {
        let Some(custom_header) = self.inner.take_custom_header(block.header.block_number) else {
            return Ok(());
        };

        tracing::debug!(
            target: "custom_header",
            block_n = block.header.block_number,
            consumed_timestamp = custom_header.timestamp,
            consumed_gas_prices = ?custom_header.gas_prices,
            block_timestamp = block.header.block_timestamp.0,
            block_gas_prices = ?block.header.gas_prices,
            replay_path,
            "consuming custom header during block close"
        );
        let is_valid = custom_header.is_block_hash_as_expected(&block_hash);
        tracing::info!(
            "replay_block_hash_verification path={} block_number={} expected_block_hash={:#x} actual_block_hash={:#x} match={} parent_block_hash={:#x} state_root={:#x} transaction_commitment={:#x} event_commitment={:#x} receipt_commitment={:#x} state_diff_commitment={:#x} timestamp={} eth_l1_gas_price={} eth_l1_data_gas_price={} eth_l2_gas_price={} strk_l1_gas_price={} strk_l1_data_gas_price={} strk_l2_gas_price={}",
            replay_path,
            block.header.block_number,
            custom_header.expected_block_hash,
            block_hash,
            is_valid,
            parent_block_hash,
            global_state_root,
            commitments.transaction.transaction_commitment,
            commitments.event.events_commitment,
            commitments.transaction.receipt_commitment,
            commitments.state_diff.state_diff_commitment,
            custom_header.timestamp,
            custom_header.gas_prices.eth_l1_gas_price,
            custom_header.gas_prices.eth_l1_data_gas_price,
            custom_header.gas_prices.eth_l2_gas_price,
            custom_header.gas_prices.strk_l1_gas_price,
            custom_header.gas_prices.strk_l1_data_gas_price,
            custom_header.gas_prices.strk_l2_gas_price
        );
        if is_valid {
            return Ok(());
        }

        let message = format!(
            "Block hash mismatch at block #{}: expected={}, computed={}. No data has been persisted.",
            block.header.block_number, custom_header.expected_block_hash, block_hash,
        );
        tracing::warn!(
            target: "custom_header",
            block_n = block.header.block_number,
            expected = ?custom_header.expected_block_hash,
            computed = ?block_hash,
            state_root = ?global_state_root,
            "{message}"
        );
        bail!("{message}")
    }

    /// Clears replay headers that cannot apply after this block has closed successfully.
    ///
    /// The cleanup runs after hash verification and before any block-part persistence.
    fn clear_consumed_custom_headers(&self, block_n: u64, replay_path: &'static str) {
        let cleared_headers = self.inner.clear_custom_headers_through(block_n);
        if cleared_headers > 0 {
            tracing::debug!(
                target: "custom_header",
                block_n,
                cleared_headers,
                replay_path,
                "cleared staged custom headers through closed block"
            );
        }
    }

    /// Builds and validates the confirmed header and records block-hash computation time.
    ///
    /// No block parts are persisted until this shared metadata phase succeeds.
    fn prepare_confirmed_header(
        &self,
        block: &FullBlockWithoutCommitments,
        commitments: &BlockCommitments,
        parent_block_hash: Felt,
        global_state_root: Felt,
        pre_v0_13_2_hash_override: bool,
        replay_path: &'static str,
        timings: &mut CloseBlockTimings,
    ) -> Result<(mp_block::header::Header, Felt)> {
        let header =
            block.header.clone().into_confirmed_header(parent_block_hash, commitments.clone(), global_state_root);
        let started_at = Instant::now();
        let block_hash = header.compute_hash(self.inner.chain_config.chain_id.to_felt(), pre_v0_13_2_hash_override);
        timings.block_hash_compute = started_at.elapsed();
        let elapsed = timings.block_hash_compute.as_secs_f64();
        metrics().block_hash_compute_duration.record(elapsed, &[]);
        metrics().block_hash_compute_last.record(elapsed, &[]);

        tracing::debug!(
            block_number = block.header.block_number,
            block_hash = format_args!("{block_hash:#x}"),
            replay_path,
            "computed block hash"
        );
        self.verify_custom_header(block, commitments, parent_block_hash, global_state_root, block_hash, replay_path)?;
        self.clear_consumed_custom_headers(block.header.block_number, replay_path);
        Ok((header, block_hash))
    }

    /// Copies worker-produced Merkle timings into the close result.
    ///
    /// This keeps the parallel close path's timing schema identical to the inline path.
    fn apply_precomputed_merklization_timings(
        timings: &mut CloseBlockTimings,
        merklization: rocksdb::global_trie::MerklizationTimings,
    ) {
        timings.merklization = merklization.total;
        timings.contract_trie_root = merklization.contract_trie_root;
        timings.class_trie_root = merklization.class_trie_root;
        timings.contract_storage_trie_commit = merklization.contract_trie.storage_commit;
        timings.contract_trie_commit = merklization.contract_trie.trie_commit;
        timings.class_trie_commit = merklization.class_trie.trie_commit;
    }

    /// Commits staged inline trie changes and records their detailed timing fields.
    ///
    /// This is called only after replay hash verification has accepted the block.
    fn commit_staged_tries(
        &self,
        block_n: u64,
        staged_tries: rocksdb::global_trie::StagedGlobalTries,
        merklization_started_at: Instant,
        timings: &mut CloseBlockTimings,
    ) -> Result<()> {
        let contract_trie_root_duration = staged_tries.contract_trie_root_duration;
        let class_trie_root_duration = staged_tries.class_trie_root_duration;
        let (contract_trie_timings, class_trie_timings) = staged_tries.commit(block_n)?;
        let merklization_duration = merklization_started_at.elapsed();

        metrics().apply_to_global_trie_duration.record(merklization_duration.as_secs_f64(), &[]);
        metrics().apply_to_global_trie_last.record(merklization_duration.as_secs_f64(), &[]);
        metrics().contract_trie_root_duration.record(contract_trie_root_duration.as_secs_f64(), &[]);
        metrics().contract_trie_root_last.record(contract_trie_root_duration.as_secs_f64(), &[]);
        metrics().class_trie_root_duration.record(class_trie_root_duration.as_secs_f64(), &[]);
        metrics().class_trie_root_last.record(class_trie_root_duration.as_secs_f64(), &[]);

        timings.merklization = merklization_duration;
        timings.contract_trie_root = contract_trie_root_duration;
        timings.class_trie_root = class_trie_root_duration;
        timings.contract_storage_trie_commit = contract_trie_timings.storage_commit;
        timings.contract_trie_commit = contract_trie_timings.trie_commit;
        timings.class_trie_commit = class_trie_timings.trie_commit;
        Ok(())
    }

    /// Persists every confirmed block part and records the aggregate database-write duration.
    ///
    /// Callers must complete root computation and replay-header verification before invoking it.
    fn write_confirmed_block_parts(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        header: mp_block::header::Header,
        block_hash: Felt,
        timings: &mut CloseBlockTimings,
    ) -> Result<()> {
        let started_at = Instant::now();
        self.write_header(BlockHeaderWithSignatures { header, block_hash, consensus_signatures: vec![] })?;
        self.write_transactions(block.header.block_number, &block.transactions)?;
        self.write_state_diff(block.header.block_number, &block.state_diff)?;
        self.write_events(block.header.block_number, &block.events)?;
        self.write_classes(block.header.block_number, classes)?;
        timings.db_write_block_parts = started_at.elapsed();
        let elapsed = timings.db_write_block_parts.as_secs_f64();
        metrics().db_write_block_parts_duration.record(elapsed, &[]);
        metrics().db_write_block_parts_last.record(elapsed, &[]);
        Ok(())
    }

    /// Computes and persists a confirmed block using the inline staged-trie path.
    ///
    /// The confirmed head is deliberately unchanged; the caller publishes it separately.
    fn write_new_confirmed_inner(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
        get_full_block_with_classes_duration: Duration,
    ) -> Result<AddFullBlockResult> {
        let mut timings = CloseBlockTimings {
            get_full_block_with_classes: get_full_block_with_classes_duration,
            ..Default::default()
        };
        let parent_block_hash = self.parent_block_hash()?;
        let commitments = self.compute_block_commitments(block, &mut timings);

        let merklization_started_at = Instant::now();
        let (global_state_root, staged_tries) = self.inner.db.compute_global_trie_staged(
            &block.state_diff,
            block.header.protocol_version,
            block.header.block_number,
        )?;
        let (header, block_hash) = self.prepare_confirmed_header(
            block,
            &commitments,
            parent_block_hash,
            global_state_root,
            pre_v0_13_2_hash_override,
            "inline_trie",
            &mut timings,
        )?;
        self.commit_staged_tries(block.header.block_number, staged_tries, merklization_started_at, &mut timings)?;
        self.write_confirmed_block_parts(block, classes, header, block_hash, &mut timings)?;

        Ok(AddFullBlockResult {
            new_state_root: global_state_root,
            commitments,
            block_hash,
            parent_block_hash,
            timings,
        })
    }

    /// Persists a confirmed block whose state root was prepared by a parallel worker.
    ///
    /// This path shares commitment, hash-validation, and block-write phases with inline closing.
    fn write_new_confirmed_with_precomputed_root(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
        precomputed_root: Felt,
        merklization_timings: rocksdb::global_trie::MerklizationTimings,
        get_full_block_with_classes_duration: Duration,
    ) -> Result<AddFullBlockResult> {
        let mut timings = CloseBlockTimings {
            get_full_block_with_classes: get_full_block_with_classes_duration,
            ..Default::default()
        };
        let parent_block_hash = self.parent_block_hash()?;
        let commitments = self.compute_block_commitments(block, &mut timings);
        Self::apply_precomputed_merklization_timings(&mut timings, merklization_timings);
        let (header, block_hash) = self.prepare_confirmed_header(
            block,
            &commitments,
            parent_block_hash,
            precomputed_root,
            pre_v0_13_2_hash_override,
            "parallel_precomputed",
            &mut timings,
        )?;
        self.write_confirmed_block_parts(block, classes, header, block_hash, &mut timings)?;

        Ok(AddFullBlockResult { new_state_root: precomputed_root, commitments, block_hash, parent_block_hash, timings })
    }

    /// Write preconfirmed block parts with a precomputed state root from a parallel worker.
    ///
    /// This does NOT advance the confirmed head. Callers that need to confirm must invoke
    /// `new_confirmed_block` only after any required durability steps (e.g. boundary flush)
    /// complete successfully.
    pub fn write_preconfirmed_with_precomputed_root(
        &self,
        pre_v0_13_2_hash_override: bool,
        block_n: u64,
        state_diff: StateDiff,
        precomputed_root: Felt,
        merklization_timings: rocksdb::global_trie::MerklizationTimings,
    ) -> Result<AddFullBlockResult> {
        let fetch_start = Instant::now();
        let preconfirmed_view = self
            .inner
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("There is no preconfirmed block #{block_n}"))?;
        let (mut block, classes) = preconfirmed_view.get_full_block_without_state_diff()?;
        let fetch_duration = fetch_start.elapsed();
        let fetch_secs = fetch_duration.as_secs_f64();
        metrics().get_full_block_without_state_diff_duration.record(fetch_secs, &[]);
        metrics().get_full_block_without_state_diff_last.record(fetch_secs, &[]);

        block.state_diff = state_diff;

        self.write_new_confirmed_with_precomputed_root(
            &block,
            &classes,
            pre_v0_13_2_hash_override,
            precomputed_root,
            merklization_timings,
            fetch_duration,
        )
    }

    /// Close a preconfirmed block with a precomputed state root from a parallel worker.
    /// This is the parallel merkle counterpart of `close_preconfirmed`.
    #[deprecated(
        note = "Use write_preconfirmed_with_precomputed_root + new_confirmed_block in phased order to preserve boundary durability semantics."
    )]
    pub fn close_preconfirmed_with_precomputed_root(
        &self,
        pre_v0_13_2_hash_override: bool,
        block_n: u64,
        state_diff: StateDiff,
        precomputed_root: Felt,
        merklization_timings: rocksdb::global_trie::MerklizationTimings,
    ) -> Result<AddFullBlockResult> {
        let result = self.write_preconfirmed_with_precomputed_root(
            pre_v0_13_2_hash_override,
            block_n,
            state_diff,
            precomputed_root,
            merklization_timings,
        )?;
        self.new_confirmed_block(block_n)?;
        Ok(result)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_header(&self, header: BlockHeaderWithSignatures) -> Result<()> {
        self.inner.db.write_header(header)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()> {
        self.inner.db.write_transactions(block_n, txs)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        self.inner.db.write_state_diff(block_n, value)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_bouncer_weights(&self, block_n: u64, value: &BouncerWeights) -> Result<()> {
        self.inner.db.write_bouncer_weights(block_n, value)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_events(&self, block_n: u64, txs: &[EventWithTransactionHash]) -> Result<()> {
        self.inner.db.write_events(block_n, txs)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        self.inner.db.write_classes(block_n, converted_classes)
    }

    /// Update the compiled_class_hash_v2 (BLAKE hash) for existing classes (SNIP-34 migration).
    /// This updates the ClassInfo stored in the database with the new v2 hash.
    pub fn update_class_v2_hashes(&self, migrations: Vec<(Felt, Felt)>) -> Result<()> {
        self.inner.db.update_class_v2_hashes(migrations)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// Write a state diff to the global tries.
    /// Returns the new state root.
    ///
    /// **Warning**: The caller must ensure no block parts are saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
        protocol_version: mp_chain_config::StarknetVersion,
    ) -> Result<(Felt, rocksdb::global_trie::MerklizationTimings)> {
        self.inner.db.apply_to_global_trie(start_block_n, state_diffs, protocol_version)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    /// This function in particular marks a fully imported block as confirmed. It also clears the current preconfirmed block, if any.
    ///
    /// **Warning**: The caller must ensure this new imported block is the one following the current confirmed block.
    /// You are not allowed to call this function with earlier or later blocks.
    /// In addition, you must have fully imported the block using the low level writing primitives for each of the block
    /// parts.
    pub fn new_confirmed_block(&self, block_number: u64) -> Result<()> {
        // Flush the most latest state to db to reduce data loss
        if self
            .inner
            .config
            .flush_every_n_blocks
            .is_some_and(|flush_every_n_blocks| block_number.checked_rem(flush_every_n_blocks) == Some(0))
        {
            tracing::debug!("Flushing.");
            let started_at = Instant::now();
            self.inner.db.flush().context("Periodic database flush")?;
            warn_if_confirmed_head_phase_slow(block_number, "periodic_flush", started_at.elapsed());
        }

        // Update snapshots for storage proofs. (TODO (heemank 10/11/2025): decouple this logic)
        let started_at = Instant::now();
        self.inner.db.on_new_confirmed_head(block_number)?;
        warn_if_confirmed_head_phase_slow(block_number, "storage_head_update", started_at.elapsed());

        // Persist and publish the canonical head transition.
        let started_at = Instant::now();
        self.transition_to_confirmed_or_empty(Some(block_number))?;
        warn_if_confirmed_head_phase_slow(block_number, "head_transition", started_at.elapsed());
        // L1 pending/consumed state is a derived projection. Update it only after the durable
        // canonical head says this block is confirmed; startup re-applies this idempotently if
        // the process stops between these two operations.
        let started_at = Instant::now();
        self.inner.db.confirm_l1_messages_in_block(block_number)?;
        warn_if_confirmed_head_phase_slow(block_number, "l1_message_projection", started_at.elapsed());
        // Confirmed-path immediate GC for block-keyed preconfirmed persistence.
        let started_at = Instant::now();
        self.inner.db.delete_preconfirmed_rows_up_to(block_number)?;
        warn_if_confirmed_head_phase_slow(block_number, "preconfirmed_gc", started_at.elapsed());

        Ok(())
    }

    // /// Returns the total storage size
    // pub fn update_metrics(&self) -> u64 {
    //     self.db_metrics.update(&self.db)
    // }
}
