use crate::{
    import::BlockImporter,
    metrics::control_metrics,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
    probe::ThrottledRepeatedFuture,
};
use anyhow::Context;
use blockifier::bouncer::BouncerWeights;
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    MadaraBackend, MadaraStorageWrite,
};
use mc_gateway_client::{BlockId, GatewayProvider};
use mp_block::{header::PreconfirmedHeader, BlockHeaderWithSignatures, FullBlock, Header};
use mp_convert::Felt;
use mp_gateway::{
    block::ProviderBlockPreConfirmed,
    error::{SequencerError, StarknetErrorCode},
};
use mp_state_update::StateDiff;
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use mp_utils::AbortOnDrop;
use std::{
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub type GatewayBlockSync = PipelineController<GatewaySyncSteps>;
#[allow(clippy::too_many_arguments)]
pub fn block_with_state_update_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    starting_block_n: u64,
    parallelization: usize,
    batch_size: usize,
    keep_pre_v0_13_2_hashes: bool,
    sync_bouncer_config: bool,
    disable_reorg: bool,
) -> GatewayBlockSync {
    PipelineController::new(
        GatewaySyncSteps {
            _backend: backend,
            importer,
            client,
            keep_pre_v0_13_2_hashes,
            sync_bouncer_config,
            disable_reorg,
            reorg_guard: Arc::new(Mutex::new(())),
        },
        parallelization,
        batch_size,
        starting_block_n,
    )
}

#[derive(Clone, Copy)]
enum PreconfirmedUpdateMode {
    Ignore,
    Replace,
    Append { common_prefix: usize },
}

/// Repeats the gateway request whenever the confirmed head advances underneath it.
/// The returned block number therefore corresponds to the head observed by the completed request.
async fn fetch_current_preconfirmed(
    client: &GatewayProvider,
    backend: &Arc<MadaraBackend>,
) -> (Result<ProviderBlockPreConfirmed, SequencerError>, u64) {
    let mut subscription = backend.watch_chain_head_state();
    loop {
        let block_number = subscription.current().confirmed_tip.map(|n| n + 1).unwrap_or(/* genesis */ 0);
        tracing::debug!("Sync Get Preconfirmed block #{block_number}.");
        tokio::select! {
            biased;
            _ = subscription.recv() => continue,
            preconfirmed = client.get_preconfirmed_block(block_number) => return (preconfirmed, block_number),
        }
    }
}

/// Compares an incoming gateway preconfirmed block with the current durable projection.
/// The result distinguishes a no-op, full replacement, and suffix-only append.
fn preconfirmed_update_mode(
    backend: &Arc<MadaraBackend>,
    block: &ProviderBlockPreConfirmed,
    block_number: u64,
    header: &PreconfirmedHeader,
    n_executed: usize,
) -> PreconfirmedUpdateMode {
    let Some(mut in_backend) = backend.block_view_on_current_preconfirmed() else {
        return PreconfirmedUpdateMode::Replace;
    };
    in_backend.refresh_with_candidates();
    if in_backend.block_number() != block_number {
        return PreconfirmedUpdateMode::Ignore;
    }

    let is_replacement = in_backend.header() != header
        || in_backend.num_executed_transactions() > n_executed
        || Iterator::ne(
            in_backend.borrow_content().executed_transactions().map(|tx| tx.transaction.receipt.transaction_hash()),
            block.transactions[..n_executed].iter().map(|tx| tx.transaction_hash()),
        );
    if is_replacement {
        return PreconfirmedUpdateMode::Replace;
    }

    let common_prefix = in_backend.num_executed_transactions();
    let candidates_match = Iterator::eq(
        in_backend.candidate_transactions().iter().map(|tx| &tx.hash),
        block.transactions[n_executed..].iter().map(|tx| tx.transaction_hash()),
    );
    if common_prefix == n_executed && candidates_match {
        PreconfirmedUpdateMode::Ignore
    } else {
        PreconfirmedUpdateMode::Append { common_prefix }
    }
}

/// Converts and persists the changed portion of one gateway preconfirmed block.
/// Replacement writes start at transaction zero; append writes preserve the common executed prefix.
fn persist_preconfirmed_update(
    backend: &Arc<MadaraBackend>,
    block: ProviderBlockPreConfirmed,
    header: PreconfirmedHeader,
    mode: PreconfirmedUpdateMode,
) -> anyhow::Result<Option<()>> {
    let skip_first_n = match mode {
        PreconfirmedUpdateMode::Ignore => return Ok(None),
        PreconfirmedUpdateMode::Replace => 0,
        PreconfirmedUpdateMode::Append { common_prefix } => common_prefix,
    };
    let arrived_at = TxTimestamp::now();
    let (executed, candidates) = block.into_transactions(skip_first_n);
    let executed: Vec<_> = executed
        .into_iter()
        .map(|(transaction, state_diff)| PreconfirmedExecutedTransaction {
            transaction,
            state_diff,
            declared_class: None,
            arrived_at,
            paid_fee_on_l1: None,
        })
        .collect();
    let candidates: Vec<_> = candidates
        .into_iter()
        .map(|transaction| {
            ValidatedTransaction {
                transaction: transaction.transaction.transaction,
                paid_fee_on_l1: None,
                contract_address: transaction.contract_address,
                arrived_at,
                declared_class: None,
                hash: transaction.transaction.hash,
                charge_fee: true,
            }
            .into()
        })
        .collect();

    tracing::debug!(
        "Gateway preconfirmed block sync: skip_first_n={skip_first_n} {header:?} {executed:?}, {candidates:?}"
    );
    match mode {
        PreconfirmedUpdateMode::Replace => backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(header, executed, candidates))?,
        PreconfirmedUpdateMode::Append { .. } => {
            backend.write_access().append_to_preconfirmed(header.block_number, &executed, candidates)?
        }
        PreconfirmedUpdateMode::Ignore => unreachable!("ignore mode returns before conversion"),
    }
    Ok(Some(()))
}

/// Performs one observable gateway-preconfirmed synchronization attempt.
/// Missing or incompatible upstream data is treated as a throttled no-op, matching the polling contract.
async fn sync_gateway_preconfirmed_once(
    client: &GatewayProvider,
    backend: &Arc<MadaraBackend>,
) -> anyhow::Result<Option<()>> {
    let (block, block_number) = fetch_current_preconfirmed(client, backend).await;
    let block = match block {
        Ok(block) => block,
        Err(SequencerError::StarknetError(err)) if err.code == StarknetErrorCode::BlockNotFound => {
            tracing::debug!("Preconfirmed block #{block_number} not found.");
            return Ok(None);
        }
        Err(other) => {
            tracing::warn!("Error while getting the pre-confirmed block #{block_number} from the gateway: {other:#}");
            return Ok(None);
        }
    };
    tracing::debug!("Got Preconfirmed block #{block_number}.");

    let n_executed = block.num_executed_transactions();
    let header = block.header(block_number)?;
    let mode = preconfirmed_update_mode(backend, &block, block_number, &header, n_executed);
    persist_preconfirmed_update(backend, block, header, mode)
}

// TODO: check that the headers follow each other
pub struct GatewaySyncSteps {
    _backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    keep_pre_v0_13_2_hashes: bool,
    sync_bouncer_config: bool,
    disable_reorg: bool,
    reorg_guard: Arc<Mutex<()>>,
}

impl GatewaySyncSteps {
    /// Fetches one complete gateway block and its optional Madara bouncer weights.
    /// Both requests are contextualized with the block number for actionable failures.
    async fn fetch_block(&self, block_n: u64) -> anyhow::Result<(FullBlock, Option<BouncerWeights>)> {
        tracing::debug!("📥 Fetching block #{} from gateway", block_n);
        let block = self
            .client
            .get_state_update_with_block(BlockId::Number(block_n))
            .await
            .with_context(|| format!("Getting state update with block_n={block_n}"))?;
        let bouncer_weights = if self.sync_bouncer_config {
            Some(
                self.client
                    .get_block_bouncer_weights(block_n)
                    .await
                    .with_context(|| format!("Getting bouncer weights with block_n={block_n}"))?,
            )
        } else {
            None
        };

        Ok((block.into_full_block().context("Parsing gateway block")?, bouncer_weights))
    }

    /// Verifies an existing local genesis against the fetched upstream genesis.
    /// Returns true when the matching local block means importing block zero can be skipped.
    async fn verify_existing_genesis(&self, gateway_block: &FullBlock) -> anyhow::Result<bool> {
        let Some(local_genesis_view) = self._backend.block_view_on_confirmed(0) else {
            return Ok(false);
        };
        let local_genesis_hash = local_genesis_view.get_block_info()?.block_hash;
        let upstream_genesis_hash = gateway_block.block_hash;

        if local_genesis_hash != upstream_genesis_hash {
            control_metrics().genesis_mismatch_total.add(1, &[]);
            tracing::warn!(
                local_genesis_hash = format!("{local_genesis_hash:#x}"),
                upstream_genesis_hash = format!("{upstream_genesis_hash:#x}"),
                "sync_genesis_mismatch_detected"
            );
            self.handle_genesis_mismatch().await?;
            anyhow::bail!("Genesis mismatch resolved - database cleared, restarting sync from upstream genesis");
        }

        tracing::debug!("✅ Genesis block already exists and matches upstream, skipping block 0");
        Ok(true)
    }

    /// Processes a detected parent mismatch under the shared reorg guard.
    /// Every successful recovery aborts this pipeline so it restarts from the repaired head.
    async fn process_parent_mismatch(
        &self,
        block_n: u64,
        incoming_parent_hash: Felt,
        local_parent_hash: Felt,
    ) -> anyhow::Result<()> {
        control_metrics().reorg_detected_total.add(1, &[]);
        tracing::warn!(
            "🔄 REORG DETECTED: Parent hash mismatch at block_n={}! incoming_parent={:#x}, our_parent={:#x}",
            block_n,
            incoming_parent_hash,
            local_parent_hash
        );

        if self.disable_reorg {
            control_metrics().reorg_required_but_disabled_total.add(1, &[]);
            tracing::error!(
                block_number = block_n,
                expected_parent_hash = format!("{local_parent_hash:#x}"),
                incoming_parent_hash = format!("{incoming_parent_hash:#x}"),
                "sync_reorg_required_but_disabled"
            );
            anyhow::bail!(
                "Reorg required but disabled by config. Parent hash mismatch at block {}: expected {:#x}, got {:#x}",
                block_n,
                local_parent_hash,
                incoming_parent_hash
            );
        }

        let _reorg_guard = self.reorg_guard.lock().await;
        let common_ancestor_hash = match self.find_common_ancestor(block_n - 1).await {
            Ok(hash) => hash,
            Err(error) => {
                tracing::error!("Failed to find common ancestor: {}", error);
                return Err(error);
            }
        };
        if common_ancestor_hash == Felt::ZERO {
            tracing::warn!("sync_genesis_mismatch_recovery_required");
            self.handle_genesis_mismatch().await?;
            anyhow::bail!("Genesis mismatch resolved - database cleared, restarting sync from upstream genesis");
        }

        tracing::info!("🔄 Triggering reorg to common ancestor hash={:#x}", common_ancestor_hash);
        self._backend.revert_to(&common_ancestor_hash)?;
        self._backend.db.flush()?;
        control_metrics().reorg_processed_total.add(1, &[]);
        tracing::info!(
            "✅ Reorg completed successfully, head projection cache refreshed, aborting pipeline to restart from new head projection"
        );
        anyhow::bail!("Reorg detected and processed, restarting sync from new head projection");
    }

    /// Validates that a fetched block extends the locally confirmed parent when available.
    /// Missing parents are tolerated only for normal parallel-fetch gaps, never at the resume boundary.
    async fn verify_parent(
        &self,
        block_n: u64,
        gateway_block: &FullBlock,
        confirmed_tip_at_start: Option<u64>,
    ) -> anyhow::Result<()> {
        if block_n == 0 {
            return Ok(());
        }

        match self._backend.block_view_on_confirmed(block_n - 1) {
            Some(parent_view) => {
                let local_parent_hash = parent_view.get_block_info()?.block_hash;
                let incoming_parent_hash = gateway_block.header.parent_block_hash;
                if incoming_parent_hash != local_parent_hash {
                    self.process_parent_mismatch(block_n, incoming_parent_hash, local_parent_hash).await?;
                }
            }
            None => {
                let is_first_block_after_confirmed =
                    confirmed_tip_at_start.map(|tip| block_n - 1 == tip).unwrap_or(false);
                if is_first_block_after_confirmed {
                    tracing::error!(
                        "❌ SYNC RESUME VALIDATION FAILED: Parent block #{} should be confirmed (head projection) but not found by block_view() when fetching block #{}",
                        block_n - 1,
                        block_n
                    );
                    anyhow::bail!(
                        "Database inconsistency: Head projection indicates block {} is confirmed, but block_view() cannot find it",
                        block_n - 1
                    );
                }
                tracing::debug!(
                    "Parent block {} not yet confirmed when fetching block {} (parallel fetch gap, expected)",
                    block_n - 1,
                    block_n
                );
            }
        }
        Ok(())
    }

    /// Verifies and persists every component of one fetched gateway block on the importer pool.
    /// The returned state diff is forwarded unchanged to the pipeline's ordered stage.
    async fn import_block(
        &self,
        block_n: u64,
        gateway_block: FullBlock,
        bouncer_weights: Option<BouncerWeights>,
    ) -> anyhow::Result<StateDiff> {
        let keep_pre_v0_13_2_hashes = self.keep_pre_v0_13_2_hashes;
        self.importer
            .run_in_rayon_pool(move |importer| {
                let mut signed_header = BlockHeaderWithSignatures {
                    header: gateway_block.header,
                    block_hash: gateway_block.block_hash,
                    consensus_signatures: vec![],
                };
                let allow_pre_v0_13_2 = true;
                let state_diff_commitment = importer.verify_state_diff(
                    block_n,
                    &gateway_block.state_diff,
                    &signed_header.header,
                    allow_pre_v0_13_2,
                )?;
                let (transaction_commitment, receipt_commitment) = importer.verify_transactions(
                    block_n,
                    &gateway_block.transactions,
                    &signed_header.header,
                    allow_pre_v0_13_2,
                )?;
                let event_commitment =
                    importer.verify_events(block_n, &gateway_block.events, &signed_header.header, allow_pre_v0_13_2)?;
                if !keep_pre_v0_13_2_hashes {
                    signed_header.header = Header {
                        state_diff_commitment: Some(state_diff_commitment),
                        transaction_commitment,
                        event_commitment,
                        receipt_commitment: Some(receipt_commitment),
                        ..signed_header.header
                    };
                }
                importer.verify_header(block_n, &signed_header)?;
                importer.save_header(block_n, signed_header)?;
                if let Some(bouncer_weights) = bouncer_weights {
                    importer.save_bouncer_weights(block_n, bouncer_weights)?;
                }
                importer.save_state_diff(block_n, gateway_block.state_diff.clone())?;
                importer.save_transactions(block_n, gateway_block.transactions)?;
                importer.save_events(block_n, gateway_block.events)?;
                tracing::debug!("✅ Block #{} saved: header, state_diff, transactions, events", block_n);
                anyhow::Ok(gateway_block.state_diff)
            })
            .await
            .with_context(|| format!("Verifying block for block_n={block_n:?}"))
    }

    /// Finds the common ancestor block hash between the local chain and gateway during a reorg.
    ///
    /// This function walks backwards from a given block number, comparing local block hashes
    /// with gateway block hashes until it finds a matching hash, which indicates the common
    /// ancestor (the last valid block before the fork).
    ///
    /// # Arguments
    ///
    /// * `starting_block_n` - The block number to start searching backwards from (typically block_n - 1
    ///   when a parent hash mismatch is detected)
    ///
    /// # Returns
    ///
    /// Returns the block hash of the common ancestor.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Unable to fetch blocks from the gateway
    /// * Unable to read blocks from local storage
    /// * Genesis block is not confirmed
    /// * No common ancestor is found (should never happen in practice)
    ///
    /// 1. Start from `starting_block_n` and walk backwards
    /// 2. For each block:
    ///    - Get our local block hash
    ///    - Fetch the same block from gateway
    ///    - Compare the hashes
    /// 3. If hashes match, we found the common ancestor
    /// 4. If we reach genesis (block 0), use it as the common ancestor
    async fn find_common_ancestor(&self, starting_block_n: u64) -> anyhow::Result<mp_convert::Felt> {
        tracing::info!("🔍 Finding common ancestor starting from block {}", starting_block_n);
        let search_started_at = Instant::now();

        let mut probe_block_n = starting_block_n;

        loop {
            if probe_block_n == 0 {
                // At genesis - VERIFY it matches upstream to detect network misconfiguration
                tracing::info!("🔍 Reached genesis block, verifying against upstream...");

                let local_genesis_view = self
                    ._backend
                    .block_view_on_confirmed(0)
                    .ok_or_else(|| anyhow::anyhow!("Genesis block not found"))?;
                let local_genesis_info = local_genesis_view.get_block_info()?;
                let local_genesis_hash = local_genesis_info.block_hash;

                // Fetch upstream genesis to compare
                match self.client.get_state_update_with_block(BlockId::Number(0)).await {
                    Ok(gateway_response) => {
                        let upstream_genesis =
                            gateway_response.into_full_block().context("Parsing upstream genesis block")?;
                        let upstream_genesis_hash = upstream_genesis.block_hash;

                        if local_genesis_hash != upstream_genesis_hash {
                            control_metrics().genesis_mismatch_total.add(1, &[]);
                            control_metrics()
                                .common_ancestor_search_duration
                                .record(search_started_at.elapsed().as_secs_f64(), &[]);
                            control_metrics().common_ancestor_distance_blocks.record(starting_block_n as f64, &[]);
                            tracing::warn!(
                                local_genesis_hash = format!("{local_genesis_hash:#x}"),
                                upstream_genesis_hash = format!("{upstream_genesis_hash:#x}"),
                                "sync_genesis_mismatch_detected"
                            );
                            return Ok(Felt::ZERO);
                        }

                        control_metrics()
                            .common_ancestor_search_duration
                            .record(search_started_at.elapsed().as_secs_f64(), &[]);
                        control_metrics().common_ancestor_distance_blocks.record(starting_block_n as f64, &[]);
                        tracing::info!(
                            "✅ Genesis blocks match (hash={:#x}), using as common ancestor",
                            local_genesis_hash
                        );
                        return Ok(local_genesis_hash);
                    }
                    Err(e) => {
                        tracing::error!("Failed to fetch upstream genesis for verification: {}", e);
                        anyhow::bail!("Cannot verify genesis block against upstream: {}", e);
                    }
                }
            }

            tracing::debug!("🔍 Probing block {} for common ancestor", probe_block_n);

            // Get what we have stored for this block
            if let Some(block_view) = self._backend.block_view_on_confirmed(probe_block_n) {
                let block_info = block_view.get_block_info()?;
                let local_block_hash = block_info.block_hash;
                tracing::debug!("🔍 Our block {} hash: {:#x}", probe_block_n, local_block_hash);

                // Fetch the same block from gateway to compare
                match self.client.get_state_update_with_block(BlockId::Number(probe_block_n)).await {
                    Ok(gateway_response) => {
                        let gateway_block = gateway_response
                            .into_full_block()
                            .with_context(|| format!("Parsing gateway block {}", probe_block_n))?;
                        let gateway_hash = gateway_block.block_hash;
                        tracing::debug!("🔍 Gateway block {} hash: {:#x}", probe_block_n, gateway_hash);

                        if local_block_hash == gateway_hash {
                            // Found common ancestor!
                            control_metrics()
                                .common_ancestor_search_duration
                                .record(search_started_at.elapsed().as_secs_f64(), &[]);
                            control_metrics()
                                .common_ancestor_distance_blocks
                                .record(starting_block_n.saturating_sub(probe_block_n) as f64, &[]);
                            tracing::info!("✅ Found common ancestor at block {}", probe_block_n);
                            return Ok(local_block_hash);
                        } else {
                            tracing::debug!("❌ Block {} hash mismatch, continuing search", probe_block_n);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("⚠️ Failed to fetch block {} from gateway: {}", probe_block_n, e);
                    }
                }
            }

            probe_block_n -= 1;
        }
    }

    /// Handles the catastrophic case where local genesis doesn't match upstream.
    ///
    /// This function wipes the entire database and prepares for resync from upstream genesis.
    /// It is called when genesis mismatch is detected and auto-recovery is enabled.
    ///
    /// # Steps
    ///
    /// 1. Remove all blocks from database (starting from block 0)
    /// 2. Reset head projection to Empty
    /// 3. Flush database changes
    /// 4. Refresh backend cache
    ///
    /// After this function completes, the database will be empty and ready to sync
    /// from the upstream genesis block.
    ///
    /// # Warning
    ///
    /// This is a destructive operation that permanently deletes all blockchain data.
    /// It should only be called when genesis mismatch is detected and the operator
    /// has explicitly enabled auto-recovery.
    async fn handle_genesis_mismatch(&self) -> anyhow::Result<()> {
        let started_at = Instant::now();
        control_metrics().genesis_recovery_total.add(1, &[]);
        tracing::warn!("sync_genesis_recovery_started");

        // Step 1: Remove ALL blocks from database
        tracing::debug!("🗑️  Removing all blocks from database...");
        self._backend
            .db
            .remove_all_blocks_starting_from(0)
            .context("Removing all blocks during genesis mismatch recovery")?;
        tracing::debug!("✅ All blocks removed successfully");

        // Step 2: Reset head projection to empty
        tracing::debug!("🗑️  Resetting head projection to empty...");
        let empty_tip = mc_db::storage::StorageHeadProjection::Empty;
        self._backend
            .db
            .replace_head_projection(&empty_tip)
            .context("Resetting head projection during genesis mismatch recovery")?;
        tracing::debug!("✅ Head projection reset to empty");

        // Step 3: Flush database to ensure persistence
        tracing::debug!("🗑️  Flushing database...");
        self._backend.db.flush().context("Flushing database after wipe")?;
        tracing::debug!("✅ Database flushed successfully");

        // Step 4: Refresh backend cache
        tracing::debug!("🔄 Refreshing backend cache...");
        self._backend.refresh_head_projection_from_db().context("Refreshing head projection after database wipe")?;
        control_metrics().genesis_recovery_duration.record(started_at.elapsed().as_secs_f64(), &[]);
        tracing::info!(recovery_ms = started_at.elapsed().as_secs_f64() * 1000.0, "sync_genesis_recovery_finished");

        Ok(())
    }
}
impl PipelineSteps for GatewaySyncSteps {
    type InputItem = ();
    type SequentialStepInput = Vec<StateDiff>;
    type Output = Vec<StateDiff>;

    async fn parallel_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        _input: Vec<Self::InputItem>,
    ) -> anyhow::Result<Self::SequentialStepInput> {
        AbortOnDrop::spawn(async move {
            let mut out = vec![];
            tracing::debug!("Gateway sync parallel step {:?}", block_range);
            let confirmed_tip_at_start = self._backend.chain_head_state().confirmed_tip;

            for block_n in block_range {
                let (gateway_block, bouncer_weights) = self.fetch_block(block_n).await?;
                if block_n == 0 && self.verify_existing_genesis(&gateway_block).await? {
                    continue;
                }
                self.verify_parent(block_n, &gateway_block, confirmed_tip_at_start).await?;
                out.push(self.import_block(block_n, gateway_block, bouncer_weights).await?);
            }
            Ok(out)
        })
        .await
    }
    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
        _target_block: Option<u64>,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>> {
        tracing::debug!("Gateway sync sequential step: {block_range:?}");
        Ok(ApplyOutcome::Success(input))
    }
}

pub fn gateway_preconfirmed_block_sync(
    client: Arc<GatewayProvider>,
    _importer: Arc<BlockImporter>,
    backend: Arc<MadaraBackend>,
) -> ThrottledRepeatedFuture<()> {
    ThrottledRepeatedFuture::new(
        move |_| {
            let client = client.clone();
            let backend = backend.clone();
            async move { sync_gateway_preconfirmed_once(&client, &backend).await }
        },
        Duration::from_millis(500),
    )
}
