use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
    probe::ThrottledRepeatedFuture,
};
use anyhow::Context;
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    MadaraBackend,
};
use mc_gateway_client::{BlockId, GatewayProvider};
use mp_block::{BlockHeaderWithSignatures, FullBlock, Header};
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
