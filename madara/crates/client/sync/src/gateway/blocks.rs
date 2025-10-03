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
use mc_gateway_client::GatewayProvider;
use mp_block::{BlockHeaderWithSignatures, BlockId, FullBlock, Header};
use mp_gateway::error::{SequencerError, StarknetErrorCode};
use mp_state_update::StateDiff;
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use mp_utils::AbortOnDrop;
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
) -> GatewayBlockSync {
    PipelineController::new(
        GatewaySyncSteps { _backend: backend, importer, client, keep_pre_v0_13_2_hashes },
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
            for block_n in block_range {
                tracing::info!("📥 Fetching block #{} from gateway", block_n);
                let block = self
                    .client
                    .get_state_update_with_block(BlockId::Number(block_n))
                    .await
                    .with_context(|| format!("Getting state update with block_n={block_n}"))?;

                let gateway_block: FullBlock = block.into_full_block().context("Parsing gateway block")?;

                // Check for parent hash mismatch (reorg detection) BEFORE processing the block
                if block_n > 0 {
                    // Try to get the parent block's info (only confirmed blocks during gateway sync)
                    match self._backend.block_view(&BlockId::Number(block_n - 1)) {
                        Ok(parent_view) => {
                            let parent_info = parent_view.get_block_info()?;
                            let incoming_parent_hash = gateway_block.header.parent_block_hash;

                            // For gateway sync, we only deal with confirmed blocks, so use as_closed()
                            if let Some(closed_parent_info) = parent_info.as_closed() {
                                let our_parent_hash = closed_parent_info.block_hash;

                                if incoming_parent_hash != our_parent_hash {
                                    tracing::warn!(
                                        "🔄 REORG DETECTED: Parent hash mismatch at block_n={}! incoming_parent={:#x}, our_parent={:#x}",
                                        block_n, incoming_parent_hash, our_parent_hash
                                    );
                                    tracing::info!("🔍 Finding common ancestor to handle reorg...");

                                    // Find common ancestor by walking back our chain and comparing with gateway
                                    let mut probe_block_n = block_n - 1;
                                    let common_ancestor_hash = loop {
                                        if probe_block_n == 0 {
                                            tracing::warn!("🔍 Reached genesis block, using it as common ancestor");
                                            let genesis_view = self._backend.block_view(&BlockId::Number(0))?;
                                            let genesis_info = genesis_view.get_block_info()?;
                                            break genesis_info.as_closed()
                                                .ok_or_else(|| anyhow::anyhow!("Genesis must be confirmed"))?
                                                .block_hash;
                                        }

                                        tracing::debug!("🔍 Probing block {} for common ancestor", probe_block_n);

                                        // Get what we have stored for this block
                                        if let Ok(block_view) = self._backend.block_view(&BlockId::Number(probe_block_n)) {
                                            let block_info = block_view.get_block_info()?;
                                            if let Some(closed_info) = block_info.as_closed() {
                                                let our_hash = closed_info.block_hash;
                                                tracing::debug!("🔍 Our block {} hash: {:#x}", probe_block_n, our_hash);

                                                // Fetch the same block from gateway to compare
                                                match self.client.get_state_update_with_block(BlockId::Number(probe_block_n)).await {
                                                    Ok(gateway_response) => {
                                                        let gateway_block = gateway_response.into_full_block()
                                                            .with_context(|| format!("Parsing gateway block {}", probe_block_n))?;
                                                        let gateway_hash = gateway_block.block_hash;
                                                        tracing::debug!("🔍 Gateway block {} hash: {:#x}", probe_block_n, gateway_hash);

                                                        if our_hash == gateway_hash {
                                                            // Found common ancestor!
                                                            tracing::info!("✅ Found common ancestor at block {}", probe_block_n);
                                                            break our_hash;
                                                        } else {
                                                            tracing::debug!("❌ Block {} hash mismatch, continuing search", probe_block_n);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!("⚠️ Failed to fetch block {} from gateway: {}", probe_block_n, e);
                                                    }
                                                }
                                            }
                                        }

                                        probe_block_n -= 1;
                                    };

                                    tracing::info!("🔄 Triggering reorg to common ancestor hash={:#x}", common_ancestor_hash);
                                    self._backend.revert_to(&common_ancestor_hash)?;

                                    // Flush database to ensure revert is persisted
                                    self._backend.db.flush()?;

                                    // Reload the backend's cached chain tip from database after reorg
                                    // The revert_to function updated the database, but the backend's cache is stale
                                    let fresh_chain_tip = self._backend.db.get_chain_tip()
                                        .context("Getting fresh chain tip after reorg")?;
                                    let backend_chain_tip = mc_db::ChainTip::from_storage(fresh_chain_tip);
                                    self._backend.chain_tip.send_replace(backend_chain_tip);
                                    tracing::info!("✅ Reorg completed successfully, chain tip cache refreshed, aborting pipeline to restart from new chain tip");

                                    // Return error to abort the entire pipeline
                                    // The sync controller will restart from the database's new tip
                                    anyhow::bail!("Reorg detected and processed, restarting sync from new chain tip");
                                }
                            }
                        }
                        Err(_) => {
                            // Parent block doesn't exist yet, which is normal for the first block
                            tracing::trace!("Parent block {} not found, continuing normally", block_n - 1);
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
                            block.transactions[..n_executed].iter().map(|tx| tx.transaction_hash())
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
