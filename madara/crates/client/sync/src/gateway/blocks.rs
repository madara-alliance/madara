use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
    probe::ThrottledRepeatedFuture,
};
use anyhow::Context;
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    MadaraBackend, MadaraStorageRead, MadaraStorageWrite,
};
use mc_gateway_client::{BlockId, GatewayProvider};
use mp_block::{BlockHeaderWithSignatures, FullBlock, Header};
use mp_gateway::error::{SequencerError, StarknetErrorCode};
use mp_state_update::StateDiff;
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use mp_utils::AbortOnDrop;
use mp_convert::Felt;
use std::{ops::Range, sync::Arc, time::Duration};

pub type GatewayBlockSync = PipelineController<GatewaySyncSteps>;
pub fn block_with_state_update_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    starting_block_n: u64,
    parallelization: usize,
    batch_size: usize,
    keep_pre_v0_13_2_hashes: bool,
    sync_bouncer_config: bool,
) -> GatewayBlockSync {
    PipelineController::new(
        GatewaySyncSteps { _backend: backend, importer, client, keep_pre_v0_13_2_hashes, sync_bouncer_config },
        parallelization,
        batch_size,
        starting_block_n,
    )
}

// TODO: check that the headers follow each other
pub struct GatewaySyncSteps {
    _backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    keep_pre_v0_13_2_hashes: bool,
    sync_bouncer_config: bool,
}

impl GatewaySyncSteps {
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

        let mut probe_block_n = starting_block_n;

        loop {
            if probe_block_n == 0 {
                // At genesis - VERIFY it matches upstream to detect network misconfiguration
                tracing::warn!("🔍 Reached genesis block, verifying against upstream...");

                let local_genesis_view = self._backend.block_view_on_confirmed(0)
                    .ok_or_else(|| anyhow::anyhow!("Genesis block not found"))?;
                let local_genesis_info = local_genesis_view.get_block_info()?;
                let local_genesis_hash = local_genesis_info.block_hash;

                // Fetch upstream genesis to compare
                match self.client.get_state_update_with_block(BlockId::Number(0)).await {
                    Ok(gateway_response) => {
                        let upstream_genesis = gateway_response
                            .into_full_block()
                            .context("Parsing upstream genesis block")?;
                        let upstream_genesis_hash = upstream_genesis.block_hash;

                        if local_genesis_hash != upstream_genesis_hash {
                            tracing::warn!("🔄 Genesis mismatch detected - starting automatic recovery");
                            tracing::warn!("🔄 Wiping database and preparing to resync from upstream...");
                            return Ok(Felt::ZERO);
                        }

                        tracing::info!("✅ Genesis blocks match (hash={:#x}), using as common ancestor", local_genesis_hash);
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
    /// 2. Reset chain tip to Empty
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
        tracing::warn!("Auto-recovery from genesis mismatch is enabled.");
        tracing::warn!("All local blockchain data will be deleted and resynced from upstream.");

        // Step 1: Remove ALL blocks from database
        tracing::info!("🗑️  Removing all blocks from database...");
        self._backend
            .db
            .remove_all_blocks_starting_from(0)
            .context("Removing all blocks during genesis mismatch recovery")?;
        tracing::info!("✅ All blocks removed successfully");

        // Step 2: Reset chain tip to empty
        tracing::info!("🗑️  Resetting chain tip to empty...");
        let empty_tip = mc_db::storage::StorageChainTip::Empty;
        self._backend
            .db
            .replace_chain_tip(&empty_tip)
            .context("Resetting chain tip during genesis mismatch recovery")?;
        tracing::info!("✅ Chain tip reset to empty");

        // Step 3: Flush database to ensure persistence
        tracing::info!("🗑️  Flushing database...");
        self._backend.db.flush().context("Flushing database after wipe")?;
        tracing::info!("✅ Database flushed successfully");

        // Step 4: Refresh backend cache
        tracing::info!("🔄 Refreshing backend cache...");
        let fresh_chain_tip = self._backend.db.get_chain_tip()
            .context("Getting fresh chain tip after database wipe")?;
        let backend_chain_tip = mc_db::ChainTip::from_storage(fresh_chain_tip);
        self._backend.chain_tip.send_replace(backend_chain_tip);
        tracing::info!("✅ Backend cache refreshed");
        tracing::info!("🔄 Node will now resync from upstream genesis block...");

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

            // Get the confirmed chain tip to detect sync resume scenarios
            let confirmed_tip_at_start = self._backend.chain_tip.borrow()
                .latest_confirmed_block_n();

            for block_n in block_range {
                tracing::info!("📥 Fetching block #{} from gateway", block_n);
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

                let gateway_block: FullBlock = block.into_full_block().context("Parsing gateway block")?;

                if block_n == 0 {
                    // Check if we already have a genesis block
                    if let Some(local_genesis_view) = self._backend.block_view_on_confirmed(0) {
                        let local_genesis_info = local_genesis_view.get_block_info()?;
                        let local_genesis_hash = local_genesis_info.block_hash;
                        let upstream_genesis_hash = gateway_block.block_hash;

                        if local_genesis_hash != upstream_genesis_hash {
                            tracing::warn!(
                                "🔄 GENESIS MISMATCH DETECTED: local_genesis={:#x}, upstream_genesis={:#x}",
                                local_genesis_hash, upstream_genesis_hash
                            );
                            tracing::warn!("🔄 Cannot sync chains with different genesis blocks");
                            tracing::warn!("🔄 Wiping database and preparing to resync from upstream...");

                            self.handle_genesis_mismatch().await?;
                            anyhow::bail!("Genesis mismatch resolved - database cleared, restarting sync from upstream genesis");
                        }

                        tracing::debug!("✅ Genesis block already exists and matches upstream, skipping block 0");
                        continue;
                    }
                }

                // Check for parent hash mismatch (reorg detection) BEFORE processing the block
                if block_n > 0 {
                    // Try to get the parent block's info (only confirmed blocks during gateway sync)
                    match self._backend.block_view_on_confirmed(block_n - 1) {
                        Some(parent_view) => {
                            let parent_info = parent_view.get_block_info()?;
                            let incoming_parent_hash = gateway_block.header.parent_block_hash;
                            let local_parent_hash = parent_info.block_hash;

                            if incoming_parent_hash != local_parent_hash {
                                tracing::warn!(
                                    "🔄 REORG DETECTED: Parent hash mismatch at block_n={}! incoming_parent={:#x}, our_parent={:#x}",
                                    block_n, incoming_parent_hash, local_parent_hash
                                );

                                // Try to find common ancestor
                                match self.find_common_ancestor(block_n - 1).await {
                                    Ok(common_ancestor_hash) => {
                                        if common_ancestor_hash == Felt::ZERO {
                                            // Genesis mismatch - no common ancestor found
                                            tracing::warn!("🔄 Genesis mismatch detected - starting automatic recovery");
                                            tracing::warn!("🔄 Wiping database and preparing to resync from upstream...");
                                            tracing::error!("❌ Genesis mismatch detected, aborting sync");
                                            self.handle_genesis_mismatch().await?;

                                            anyhow::bail!("Genesis mismatch resolved - database cleared, restarting sync from upstream genesis");
                                        } else {
                                            // Normal reorg - found common ancestor
                                            tracing::info!("🔄 Triggering reorg to common ancestor hash={:#x}", common_ancestor_hash);
                                            self._backend.revert_to(&common_ancestor_hash)?;

                                            self._backend.db.flush()?;

                                            let fresh_chain_tip = self._backend.db.get_chain_tip()
                                                .context("Getting fresh chain tip after reorg")?;
                                            let backend_chain_tip = mc_db::ChainTip::from_storage(fresh_chain_tip);
                                            self._backend.chain_tip.send_replace(backend_chain_tip);
                                            tracing::info!("✅ Reorg completed successfully, chain tip cache refreshed, aborting pipeline to restart from new chain tip");

                                            anyhow::bail!("Reorg detected and processed, restarting sync from new chain tip");
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to find common ancestor: {}", e);
                                        return Err(e);
                                    }
                                }
                            }
                        }
                        None => {
                            // Parent block not found via block_view() - could be written but not confirmed yet
                            // This is normal during parallel fetching, but we need to be careful on sync resume

                            // Check if this is the critical first block after sync resume
                            let is_first_block_after_confirmed = confirmed_tip_at_start
                                .map(|tip| block_n - 1 == tip)
                                .unwrap_or(false);

                            if is_first_block_after_confirmed {
                                // CRITICAL: This is the first block after the confirmed chain tip
                                // We MUST validate its parent_hash against our confirmed parent
                                // But block_view() failed, which means parent is not confirmed yet (shouldn't happen)
                                //
                                // This indicates the parent block exists but wasn't confirmed,
                                // which is a database inconsistency issue - the chain tip says block N-1 is confirmed
                                // but block_view() can't find it
                                tracing::error!(
                                    "❌ SYNC RESUME VALIDATION FAILED: Parent block #{} should be confirmed (chain tip) but not found by block_view() when fetching block #{}",
                                    block_n - 1, block_n
                                );
                                anyhow::bail!(
                                    "Database inconsistency: Chain tip indicates block {} is confirmed, but block_view() cannot find it",
                                    block_n - 1
                                );
                            }

                            // Normal parallel fetch gap - parent was written but not confirmed yet
                            // This is expected and safe during parallel fetching
                            tracing::debug!(
                                "Parent block {} not yet confirmed when fetching block {} (parallel fetch gap, expected)",
                                block_n - 1, block_n
                            );
                        }
                    }
                }

                let keep_pre_v0_13_2_hashes = self.keep_pre_v0_13_2_hashes;

                let state_diff = self
                    .importer
                    .run_in_rayon_pool(move |importer| {
                        let mut signed_header = BlockHeaderWithSignatures {
                            header: gateway_block.header,
                            block_hash: gateway_block.block_hash,
                            consensus_signatures: vec![],
                        };

                        // Allow the gateway format, which has legacy commitments.
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
                        let event_commitment = importer.verify_events(
                            block_n,
                            &gateway_block.events,
                            &signed_header.header,
                            allow_pre_v0_13_2,
                        )?;
                        if !keep_pre_v0_13_2_hashes {
                            // Fill in the header with the commitments missing in pre-v0.13.2 headers from the gateway.
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

                        tracing::info!("✅ Block #{} saved: header, state_diff, transactions, events", block_n);

                        anyhow::Ok(gateway_block.state_diff)
                    })
                    .await
                    .with_context(|| format!("Verifying block for block_n={block_n:?}"))?;
                out.push(state_diff);
            }
            Ok(out)
        })
        .await
    }
    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
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
            async move {
                // We do some shenanigans to abort the request if a new block has been imported.
                let mut subscription = backend.watch_chain_tip();
                let (block, block_number) = loop {
                    let block_number = subscription
                        .current()
                        .latest_confirmed_block_n()
                        .map(|n| n + 1)
                        .unwrap_or(/* genesis */ 0);
                    tracing::debug!("Sync Get Preconfirmed block #{block_number}.");
                    tokio::select! {
                        biased;
                        _ = subscription.recv() => continue, // Abort request and restart
                        preconfirmed = client.get_preconfirmed_block(block_number) => {
                            break (preconfirmed, block_number)
                        }
                    }
                };
                let block = match block {
                    Ok(block) => block,
                    Err(SequencerError::StarknetError(err)) if err.code == StarknetErrorCode::BlockNotFound => {
                        tracing::debug!("Preconfirmed block #{block_number} not found.");
                        return Ok(None);
                    }
                    Err(other) => {
                        // non-compliant gateway?
                        tracing::warn!(
                            "Error while getting the pre-confirmed block #{block_number} from the gateway: {other:#}"
                        );
                        return Ok(None);
                    }
                };

                tracing::debug!("Got Preconfirmed block #{block_number}.");

                let n_executed = block.num_executed_transactions();
                let header = block.header(block_number)?;

                // How many of these transactions do we already have? When None, we need to make a new pre-confirmed block.
                let mut common_prefix = None;

                if let Some(mut in_backend) = backend.block_view_on_preconfirmed() {
                    in_backend.refresh_with_candidates(); // we want to compare candidates too.

                    if in_backend.block_number() != block_number {
                        return Ok(None);
                    }

                    // True if this gateway block should be considered as a new pre-confirmed block entirely.
                    let new_preconfirmed = in_backend.header() != &header
                        || in_backend.num_executed_transactions() > n_executed
                        ||
                        // Compare hashes
                        // TODO: should we compute these hashes? probably not?
                        Iterator::ne(
                            in_backend.borrow_content().executed_transactions().map(|tx| tx.transaction.receipt.transaction_hash()),
                            block.transactions[..n_executed].iter().map(|tx| tx.transaction_hash()),
                        );

                    if !new_preconfirmed {
                        common_prefix = Some(in_backend.num_executed_transactions());
                    }

                    // Whether there was no change at all (no need to update the backend)
                    let has_not_changed = !new_preconfirmed
                        && in_backend.num_executed_transactions() == n_executed
                        // Compare candidate hashes.
                        && Iterator::eq(
                        in_backend.candidate_transactions().iter().map(|tx| &tx.hash),
                        block.transactions[n_executed..].iter().map(|tx| tx.transaction_hash()),
                    );
                    if has_not_changed {
                        return Ok(None);
                    }
                }

                let arrived_at = TxTimestamp::now();

                let (executed, candidates) =
                    block.into_transactions(/* skip_first_n */ common_prefix.unwrap_or(0));

                let executed: Vec<_> = executed
                    .into_iter()
                    .map(|(transaction, state_diff)| PreconfirmedExecutedTransaction {
                        transaction,
                        state_diff,
                        declared_class: None, // It seems we can't get the declared classes from the preconfirmed block :/
                        arrived_at,
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
                            declared_class: None, // Ditto.
                            hash: transaction.transaction.hash,
                            charge_fee: true, // keeping the default value as true for now
                        }
                        .into()
                    })
                    .collect();

                tracing::debug!("Gateay preconfirmed block sync: common_prefix={common_prefix:?} {header:?} {executed:?}, {candidates:?}");

                if common_prefix.is_none() {
                    // New preconfirmed block (replaces the current one if there is one)
                    backend
                        .write_access()
                        .new_preconfirmed(PreconfirmedBlock::new_with_content(header, executed, candidates))?;
                } else {
                    // Append to current pre-confirmed block.
                    backend
                        .write_access()
                        .append_to_preconfirmed(&executed, /* replace_candidates */ candidates)?;
                }

                Ok(Some(()))
            }
        },
        Duration::from_millis(500),
    )
}
