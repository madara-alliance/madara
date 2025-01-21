use crate::{
    controller::{ApplyOutcome, PipelineController, PipelineSteps},
    import::BlockImporter,
};
use anyhow::Context;
use mp_utils::graceful_shutdown;
use core::error;
use mc_db::MadaraBackend;
use mc_gateway::client::builder::FeederClient;
use mp_block::{BlockHeaderWithSignatures, BlockId, Header, TransactionWithReceipt};
use mp_chain_config::{StarknetVersion, StarknetVersionError};
use mp_gateway::state_update::{ProviderStateUpdateWithBlock, ProviderStateUpdateWithBlockPendingMaybe};
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use starknet_core::types::Felt;
use std::{ops::Range, sync::Arc};

mod classes;

struct GatewayBlock {
    block_hash: Felt,
    header: Header,
    state_diff: StateDiff,
    transactions: Vec<TransactionWithReceipt>,
    events: Vec<EventWithTransactionHash>,
}

#[derive(Debug, thiserror::Error)]
enum FromGatewayError {
    #[error("Transaction count is not equal to receipt count")]
    TransactionCountNotEqualToReceiptCount,
    #[error("Invalid starknet version: {0:#}")]
    StarknetVersion(#[from] StarknetVersionError),
    #[error("Unable to determine Starknet version for block {0:#x}")]
    FromMainnetStarknetVersion(Felt),
}

impl TryFrom<ProviderStateUpdateWithBlock> for GatewayBlock {
    type Error = FromGatewayError;
    fn try_from(value: ProviderStateUpdateWithBlock) -> Result<Self, Self::Error> {
        if value.block.transactions.len() != value.block.transaction_receipts.len() {
            return Err(FromGatewayError::TransactionCountNotEqualToReceiptCount);
        }
        let state_diff = mp_state_update::StateDiff::from(value.state_update.state_diff);
        Ok(GatewayBlock {
            block_hash: value.block.block_hash,
            header: Header {
                parent_block_hash: value.block.parent_block_hash,
                sequencer_address: value.block.sequencer_address.unwrap_or_default(),
                block_timestamp: value.block.timestamp,
                protocol_version: value
                    .block
                    .starknet_version
                    .as_deref()
                    .map(|version| Ok(version.parse()?))
                    .unwrap_or_else(|| {
                        StarknetVersion::try_from_mainnet_block_number(value.block.block_number)
                            .ok_or_else(|| FromGatewayError::FromMainnetStarknetVersion(value.block.block_hash))
                    })?,
                l1_gas_price: mp_block::header::GasPrices {
                    eth_l1_gas_price: value.block.l1_gas_price.price_in_wei,
                    strk_l1_gas_price: value.block.l1_gas_price.price_in_fri,
                    eth_l1_data_gas_price: value.block.l1_data_gas_price.price_in_wei,
                    strk_l1_data_gas_price: value.block.l1_data_gas_price.price_in_fri,
                },
                l1_da_mode: value.block.l1_da_mode,
                block_number: value.block.block_number,
                global_state_root: value.block.state_root,
                transaction_count: value.block.transactions.len() as u64,
                transaction_commitment: value.block.transaction_commitment,
                event_count: value.block.transaction_receipts.iter().map(|r| r.events.len() as u64).sum(),
                event_commitment: value.block.event_commitment,
                state_diff_length: Some(state_diff.len() as u64),
                state_diff_commitment: value.block.state_diff_commitment,
                receipt_commitment: value.block.receipt_commitment,
            },
            events: value
                .block
                .transaction_receipts
                .iter()
                .flat_map(|receipt| {
                    receipt
                        .events
                        .iter()
                        .cloned()
                        .map(|event| EventWithTransactionHash { transaction_hash: receipt.transaction_hash, event })
                })
                .collect(),
            transactions: value
                .block
                .transactions
                .into_iter()
                .zip(value.block.transaction_receipts)
                .map(|(transaction, receipt)| TransactionWithReceipt {
                    receipt: receipt.into_mp(&transaction),
                    transaction: transaction.into(),
                })
                .collect(),
            state_diff,
        })
    }
}

pub type GatewaySync = PipelineController<GatewaySyncSteps>;
pub fn block_with_state_update_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: FeederClient,
    parallelization: usize,
    batch_size: usize,
) -> GatewaySync {
    PipelineController::new(GatewaySyncSteps { backend, importer, client }, parallelization, batch_size)
}

// TODO: check that the headers follow each other
pub struct GatewaySyncSteps {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: FeederClient,
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
        let mut out = vec![];
        for block_n in block_range {
            let block = self
                .client
                .get_state_update_with_block(BlockId::Number(block_n))
                .await
                .with_context(|| format!("Getting state update with block_n={block_n}"))?;

            let ProviderStateUpdateWithBlockPendingMaybe::NonPending(block) = block else {
                anyhow::bail!("Asked for a block_n, got a pending one")
            };

            let gateway_block: GatewayBlock = block.try_into().context("Parsing gateway block")?;

            let state_diff = self
                .importer
                .run_in_rayon_pool(move |importer| {
                    let signed_header = BlockHeaderWithSignatures {
                        header: gateway_block.header,
                        block_hash: gateway_block.block_hash,
                        consensus_signatures: vec![],
                    };

                    importer.verify_header(block_n, &signed_header)?;
                    importer.verify_state_diff(block_n, &gateway_block.state_diff, &signed_header.header)?;
                    importer.verify_transactions(block_n, &gateway_block.transactions, &signed_header.header)?;
                    importer.verify_events(block_n, &gateway_block.events, &signed_header.header)?;

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
    }
    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>> {
        tracing::debug!("gateway sync sequential step: {block_range:?}");
        if let Some(block_n) = block_range.last() {
            self.backend.head_status().headers.set(Some(block_n));
            self.backend.head_status().state_diffs.set(Some(block_n));
            self.backend.head_status().transactions.set(Some(block_n));
            self.backend.head_status().events.set(Some(block_n));
        }
        Ok(ApplyOutcome::Success(input))
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}

pub struct ForwardSyncConfig {
    pub block_parallelization: usize,
    pub block_batch_size: usize,
    pub classes_parallelization: usize,
    pub classes_batch_size: usize,
    pub apply_state_parallelization: usize,
    pub apply_state_batch_size: usize,
}

impl Default for ForwardSyncConfig {
    fn default() -> Self {
        Self {
            block_parallelization: 2,
            block_batch_size: 1,
            classes_parallelization: 2,
            classes_batch_size: 1,
            apply_state_parallelization: 1,
            apply_state_batch_size: 1,
        }
    }
}

/// Events pipeline is currently always done after tx and receipts for now.
/// TODO: fix that when the db supports saving events separately.
pub async fn forward_sync(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: FeederClient,
    config: ForwardSyncConfig,
) -> anyhow::Result<()> {
    let mut blocks_pipeline = block_with_state_update_pipeline(
        backend.clone(),
        importer.clone(),
        client.clone(),
        config.block_parallelization,
        config.block_batch_size,
    );
    let mut classes_pipeline = classes::classes_pipeline(
        backend.clone(),
        importer.clone(),
        client.clone(),
        config.classes_parallelization,
        config.classes_batch_size,
    );
    let mut apply_state_pipeline = super::apply_state::apply_state_pipeline(
        backend.clone(),
        importer.clone(),
        config.apply_state_parallelization,
        config.apply_state_batch_size,
    );

    loop {
        while blocks_pipeline.can_schedule_more() {
            // todo follow until l1 head
            blocks_pipeline.push(std::iter::once(()))
        }

        tokio::select! {
            _ = graceful_shutdown() => break,

            Some(res) = blocks_pipeline.next(), if classes_pipeline.can_schedule_more() && apply_state_pipeline.can_schedule_more() => {
                let (_range, state_diffs) = res?;
                classes_pipeline.push(state_diffs.iter().map(|s| s.all_declared_classes()));
                apply_state_pipeline.push(state_diffs);
            }
            Some(res) = classes_pipeline.next() => {
                res?;
            }
            Some(res) = apply_state_pipeline.next() => {
                res?;
            }
            // all pipelines are empty, we're done :)
            else => break
        }
    }
    Ok(())
}
