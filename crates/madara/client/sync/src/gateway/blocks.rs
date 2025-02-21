use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
    probe::ThrottledRepeatedFuture,
    util::AbortOnDrop,
};
use anyhow::Context;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mp_block::{BlockHeaderWithSignatures, BlockId, BlockTag, FullBlock, Header, PendingFullBlock};
use mp_gateway::{
    error::{SequencerError, StarknetErrorCode},
    state_update::ProviderStateUpdateWithBlockPendingMaybe,
};
use mp_state_update::StateDiff;
use starknet_core::types::Felt;
use std::{ops::Range, sync::Arc, time::Duration};

pub type GatewayBlockSync = PipelineController<GatewaySyncSteps>;
pub fn block_with_state_update_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    parallelization: usize,
    batch_size: usize,
    keep_pre_v0_13_2_hashes: bool,
) -> GatewayBlockSync {
    PipelineController::new(
        GatewaySyncSteps { backend, importer, client, keep_pre_v0_13_2_hashes },
        parallelization,
        batch_size,
    )
}

// TODO: check that the headers follow each other
pub struct GatewaySyncSteps {
    backend: Arc<MadaraBackend>,
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
                let block = self
                    .client
                    .get_state_update_with_block(BlockId::Number(block_n))
                    .await
                    .with_context(|| format!("Getting state update with block_n={block_n}"))?;

                let ProviderStateUpdateWithBlockPendingMaybe::NonPending(block) = block else {
                    anyhow::bail!("Asked for a block_n, got a pending one")
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
                        importer.save_state_diff(block_n, gateway_block.state_diff.clone())?;
                        importer.save_transactions(block_n, gateway_block.transactions)?;
                        importer.save_events(block_n, gateway_block.events)?;

                        anyhow::Ok(gateway_block.state_diff)
                    })
                    .await?;
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
        if let Some(block_n) = block_range.last() {
            self.backend.clear_pending_block().context("Clearing pending block")?;
            self.backend.head_status().headers.set(Some(block_n));
            self.backend.head_status().state_diffs.set(Some(block_n));
            self.backend.head_status().transactions.set(Some(block_n));
            self.backend.head_status().events.set(Some(block_n));
            self.backend.save_head_status_to_db()?;
        }
        Ok(ApplyOutcome::Success(input))
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}

pub fn gateway_pending_block_sync(
    client: Arc<GatewayProvider>,
    importer: Arc<BlockImporter>,
    backend: Arc<MadaraBackend>,
) -> ThrottledRepeatedFuture<()> {
    ThrottledRepeatedFuture::new(
        move |_| {
            let client = client.clone();
            let importer = importer.clone();
            let backend = backend.clone();
            async move {
                let block = match client.get_state_update_with_block(BlockId::Tag(BlockTag::Pending)).await {
                    Ok(block) => block,
                    // Sometimes the gateway returns the latest closed block instead of the pending one, because there is no pending block.
                    // Deserialization fails in this case.
                    Err(SequencerError::DeserializeBody { .. }) => return Ok(None),
                    Err(SequencerError::StarknetError(err)) if err.code == StarknetErrorCode::BlockNotFound => {
                        return Ok(None)
                    }
                    Err(other) => {
                        // non-compliant gateway?
                        tracing::warn!("Could not parse the pending block returned by the gateway: {other:#}");
                        return Ok(None);
                    }
                };

                let ProviderStateUpdateWithBlockPendingMaybe::Pending(block) = block else {
                    tracing::debug!("Asked for a pending block, got a closed one");
                    return Ok(None);
                };

                let parent_hash = backend
                    .get_block_hash(&BlockId::Tag(BlockTag::Latest))
                    .context("Getting latest block hash")?
                    .unwrap_or(Felt::ZERO);

                if block.block.parent_block_hash != parent_hash {
                    tracing::debug!("Expected parent_hash={parent_hash:#x}, got {:#x}", block.block.parent_block_hash);
                    return Ok(None);
                }

                tracing::info!("BLOCK = {block:?}, {parent_hash:#x}");

                let block: PendingFullBlock = block.into_full_block().context("Parsing gateway pending block")?;

                let classes = super::classes::get_classes(
                    &client,
                    BlockId::Tag(BlockTag::Pending),
                    &block.state_diff.all_declared_classes(),
                )
                .await
                .context("Getting pending block classes")?;

                importer
                    .run_in_rayon_pool(move |importer| {
                        let classes =
                            importer.verify_compile_classes(None, classes, &block.state_diff.all_declared_classes())?;
                        importer.save_pending_classes(classes)?;
                        importer.save_pending_block(block)?;
                        anyhow::Ok(())
                    })
                    .await?;

                Ok(Some(()))
            }
        },
        Duration::from_secs(1),
    )
}
