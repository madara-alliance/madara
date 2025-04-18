//! Contains the code required to sync data from the feeder efficiently.
use crate::fetch::fetchers::fetch_pending_block_and_updates;
use crate::fetch::fetchers::WarpUpdateConfig;
use crate::fetch::l2_fetch_task;
use crate::fetch::L2FetchConfig;
use anyhow::Context;
use futures::{stream, StreamExt};
use mc_block_import::{
    BlockImportResult, BlockImporter, BlockValidationContext, PreValidatedBlock, UnverifiedFullBlock,
};
use mc_db::MadaraBackend;
use mc_db::MadaraStorageError;
use mc_gateway_client::GatewayProvider;
use mc_telemetry::{TelemetryHandle, VerbosityLevel};
use mp_block::BlockId;
use mp_block::BlockTag;
use mp_gateway::error::SequencerError;
use mp_sync::SyncStatusProvider;
use mp_utils::service::ServiceContext;
use mp_utils::trim_hash;
use mp_utils::PerfStopwatch;
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;
use std::pin::pin;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio::time::Duration;

// Module-level constants
// Maximum number of consecutive failures allowed in the highest block fetch task before terminating
// Delay after encountering an error when fetching the highest block
const HIGHEST_BLOCK_FETCH_ERROR_DELAY_SECS: u64 = 1;
const HIGHES_BLOCK_FETCH_INTERVAL: u64 = 10;

// TODO: add more explicit error variants
#[derive(thiserror::Error, Debug)]
pub enum L2SyncError {
    #[error("Provider error: {0:#}")]
    SequencerError(#[from] SequencerError),
    #[error("Database error: {0:#}")]
    Db(#[from] MadaraStorageError),
    #[error(transparent)]
    BlockImport(#[from] mc_block_import::BlockImportError),
    #[error("Unexpected class type for class hash {class_hash:#x}")]
    UnexpectedClassType { class_hash: Felt },
}

/// Contains the latest Starknet verified state on L2
#[derive(Debug, Clone)]
pub struct L2StateUpdate {
    pub block_number: u64,
    pub global_root: Felt,
    pub block_hash: Felt,
}

pub struct L2VerifyApplyConfig {
    block_import: Arc<BlockImporter>,
    backup_every_n_blocks: Option<u64>,
    flush_every_n_blocks: u64,
    flush_every_n_seconds: u64,
    stop_on_sync: bool,
    telemetry: Arc<TelemetryHandle>,
    validation: BlockValidationContext,
    block_conv_receiver: mpsc::Receiver<PreValidatedBlock>,
}

#[tracing::instrument(skip(backend, ctx, config), fields(module = "Sync"))]
async fn l2_verify_and_apply_task(
    backend: Arc<MadaraBackend>,
    mut ctx: ServiceContext,
    config: L2VerifyApplyConfig,
) -> anyhow::Result<()> {
    let L2VerifyApplyConfig {
        block_import,
        backup_every_n_blocks,
        flush_every_n_blocks,
        flush_every_n_seconds,
        stop_on_sync,
        telemetry,
        validation,
        mut block_conv_receiver,
    } = config;

    let mut last_block_n = 0;
    let mut instant = std::time::Instant::now();
    let target_duration = std::time::Duration::from_secs(flush_every_n_seconds);

    while let Some(Some(block)) = ctx.run_until_cancelled(pin!(block_conv_receiver.recv())).await {
        let BlockImportResult { header, block_hash } = block_import.verify_apply(block, validation.clone()).await?;

        if header.block_number - last_block_n >= flush_every_n_blocks || instant.elapsed() >= target_duration {
            last_block_n = header.block_number;
            instant = std::time::Instant::now();
            backend.flush().context("Flushing database")?;
        }

        tracing::info!(
            "✨ Imported #{} ({}) and updated state root ({})",
            header.block_number,
            trim_hash(&block_hash),
            trim_hash(&header.global_state_root)
        );
        tracing::debug!(
            "Block import #{} ({:#x}) has state root {:#x}",
            header.block_number,
            block_hash,
            header.global_state_root
        );

        telemetry.send(
            VerbosityLevel::Info,
            serde_json::json!({
                "best": block_hash.to_fixed_hex_string(),
                "height": header.block_number,
                "origin": "Own",
                "msg": "block.import",
            }),
        );

        if backup_every_n_blocks.is_some_and(|backup_every_n_blocks| header.block_number % backup_every_n_blocks == 0) {
            tracing::info!("⏳ Backing up database at block {}...", header.block_number);
            let sw = PerfStopwatch::new();
            backend.backup().await.context("backing up database")?;
            tracing::info!("✅ Database backup is done ({:?})", sw.elapsed());
        }
    }

    if stop_on_sync {
        ctx.cancel_global()
    }

    anyhow::Ok(())
}

async fn l2_block_conversion_task(
    updates_receiver: mpsc::Receiver<UnverifiedFullBlock>,
    output: mpsc::Sender<PreValidatedBlock>,
    block_import: Arc<BlockImporter>,
    validation: BlockValidationContext,
    mut ctx: ServiceContext,
) -> anyhow::Result<()> {
    // Items of this stream are futures that resolve to blocks, which becomes a regular stream of blocks
    // using futures buffered.
    let conversion_stream = stream::unfold(
        (updates_receiver, block_import, validation.clone(), ctx.clone()),
        |(mut updates_recv, block_import, validation, ctx)| async move {
            updates_recv.recv().await.map(|block| {
                let block_import_ = Arc::clone(&block_import);
                let validation_ = validation.clone();
                (
                    async move { block_import_.pre_validate(block, validation_).await },
                    (updates_recv, block_import, validation, ctx),
                )
            })
        },
    );

    let mut stream = pin!(conversion_stream.buffered(10));
    while let Some(Some(block)) = ctx.run_until_cancelled(stream.next()).await {
        if output.send(block?).await.is_err() {
            // channel closed
            break;
        }
    }

    anyhow::Ok(())
}

struct L2PendingBlockConfig {
    block_import: Arc<BlockImporter>,
    once_caught_up_receiver: oneshot::Receiver<()>,
    pending_block_poll_interval: Duration,
    validation: BlockValidationContext,
}

async fn l2_pending_block_task(
    backend: Arc<MadaraBackend>,
    provider: Arc<GatewayProvider>,
    mut ctx: ServiceContext,
    config: L2PendingBlockConfig,
) -> anyhow::Result<()> {
    let L2PendingBlockConfig { block_import, once_caught_up_receiver, pending_block_poll_interval, validation } =
        config;

    // clear pending status
    {
        backend.clear_pending_block().context("Clearing pending block")?;
        tracing::debug!("l2_pending_block_task: startup: wrote no pending");
    }

    // we start the pending block task only once the node has been fully sync
    if once_caught_up_receiver.await.is_err() {
        // channel closed
        return Ok(());
    }

    tracing::debug!("Start pending block poll");

    let mut interval = tokio::time::interval(pending_block_poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while ctx.run_until_cancelled(interval.tick()).await.is_some() {
        tracing::debug!("Getting pending block...");

        let current_block_hash = backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))
            .context("Getting latest block hash")?
            .unwrap_or(/* genesis parent block hash */ Felt::ZERO);

        let chain_id = &backend.chain_config().chain_id;
        let Some(block) = fetch_pending_block_and_updates(current_block_hash, chain_id, &provider)
            .await
            .context("Getting pending block from FGW")?
        else {
            continue;
        };

        // HACK(see issue #239): The latest block in db may not match the pending parent block hash
        // Just silently ignore it for now and move along.
        let import_block = || async {
            let block = block_import.pre_validate_pending(block, validation.clone()).await?;
            block_import.verify_apply_pending(block, validation.clone()).await?;
            anyhow::Ok(())
        };

        if let Err(err) = import_block().await {
            tracing::debug!("Failed to import pending block: {err:#}");
        }
    }

    Ok(())
}

async fn l2_highest_block_fetch(
    provider: Arc<GatewayProvider>,
    mut ctx: ServiceContext,
    sync_status_provider: SyncStatusProvider,
) -> anyhow::Result<()> {
    tracing::debug!("Start highest block poll");

    let mut interval = tokio::time::interval(Duration::from_secs(HIGHES_BLOCK_FETCH_INTERVAL));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while ctx.run_until_cancelled(interval.tick()).await.is_some() {
        match provider.get_block_with_header_only().await {
            Ok(block) => {
                // For a regular block, we have both number and hash
                let block_number = block.block_number;
                let block_hash = block.block_hash;

                // Update the sync status provider with the highest block info
                sync_status_provider.set_highest_block(block_number, block_hash).await;

                tracing::debug!("Updated highest block: number={}, hash={}", block_number, block_hash);
            }
            Err(e) => {
                tracing::error!("Error getting latest block: {:?}", e);
                tokio::time::sleep(Duration::from_secs(HIGHEST_BLOCK_FETCH_ERROR_DELAY_SECS)).await;
            }
        }
    }

    tracing::debug!("Highest block fetch service stopped");
    Ok(())
}

pub struct L2SyncConfig {
    pub first_block: u64,
    pub n_blocks_to_sync: Option<u64>,
    pub stop_on_sync: bool,
    pub sync_parallelism: u8,
    pub verify: bool,
    pub sync_polling_interval: Option<Duration>,
    pub backup_every_n_blocks: Option<u64>,
    pub flush_every_n_blocks: u64,
    pub flush_every_n_seconds: u64,
    pub pending_block_poll_interval: Duration,
    pub ignore_block_order: bool,
    pub chain_id: ChainId,
    pub telemetry: Arc<TelemetryHandle>,
    pub block_importer: Arc<BlockImporter>,
    pub warp_update: Option<WarpUpdateConfig>,
}

/// Spawns workers to fetch blocks and state updates from the feeder.
#[tracing::instrument(skip(backend, provider, ctx, config, sync_status_provider), fields(
    module = "Sync"
))]
pub async fn sync(
    backend: Arc<MadaraBackend>,
    provider: GatewayProvider,
    ctx: ServiceContext,
    config: L2SyncConfig,
    sync_status_provider: SyncStatusProvider,
) -> anyhow::Result<()> {
    let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(8);
    let (block_conv_sender, block_conv_receiver) = mpsc::channel(4);
    let provider = Arc::new(provider);
    let (once_caught_up_sender, once_caught_up_receiver) = oneshot::channel();

    // [Fetch task] ==new blocks and updates=> [Block conversion task] ======> [Verification and apply
    // task]
    // - Fetch task does parallel fetching
    // - Block conversion is compute heavy and parallel wrt. the next few blocks,
    // - Verification is sequential and does a lot of compute when state root verification is enabled.
    //   DB updates happen here too.

    // we are using separate tasks so that fetches don't get clogged up if by any chance the verify task
    // starves the tokio worker
    let validation = BlockValidationContext {
        trust_transaction_hashes: false,
        trust_global_tries: !config.verify,
        chain_id: config.chain_id,
        trust_class_hashes: false,
        ignore_block_order: config.ignore_block_order,
    };

    let mut join_set = JoinSet::new();
    let warp_update_shutdown_sender =
        config.warp_update.as_ref().map(|w| w.warp_update_shutdown_receiver).unwrap_or(false);

    join_set.spawn(l2_highest_block_fetch(Arc::clone(&provider), ctx.clone(), sync_status_provider));

    join_set.spawn(l2_fetch_task(
        Arc::clone(&backend),
        Arc::clone(&provider),
        ctx.clone(),
        L2FetchConfig {
            first_block: config.first_block,
            fetch_stream_sender,
            once_caught_up_sender,
            sync_polling_interval: config.sync_polling_interval,
            n_blocks_to_sync: config.n_blocks_to_sync,
            stop_on_sync: config.stop_on_sync,
            sync_parallelism: config.sync_parallelism as usize,
            warp_update: config.warp_update,
        },
    ));
    join_set.spawn(l2_block_conversion_task(
        fetch_stream_receiver,
        block_conv_sender,
        Arc::clone(&config.block_importer),
        validation.clone(),
        ctx.clone(),
    ));
    join_set.spawn(l2_verify_and_apply_task(
        Arc::clone(&backend),
        ctx.clone(),
        L2VerifyApplyConfig {
            block_import: Arc::clone(&config.block_importer),
            backup_every_n_blocks: config.backup_every_n_blocks,
            flush_every_n_blocks: config.flush_every_n_blocks,
            flush_every_n_seconds: config.flush_every_n_seconds,
            stop_on_sync: config.stop_on_sync || warp_update_shutdown_sender,
            telemetry: config.telemetry,
            validation: validation.clone(),
            block_conv_receiver,
        },
    ));
    join_set.spawn(l2_pending_block_task(
        Arc::clone(&backend),
        provider,
        ctx.clone(),
        L2PendingBlockConfig {
            block_import: Arc::clone(&config.block_importer),
            once_caught_up_receiver,
            pending_block_poll_interval: config.pending_block_poll_interval,
            validation: validation.clone(),
        },
    ));

    while let Some(res) = join_set.join_next().await {
        res.context("task was dropped")??;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::utils::gateway::{test_setup, TestContext};
    use mc_block_import::tests::block_import_utils::create_dummy_unverified_full_block;
    use mc_block_import::BlockImporter;
    use mc_db::{db_block_id::DbBlockId, MadaraBackend};

    use mc_telemetry::TelemetryService;
    use mp_block::header::L1DataAvailabilityMode;
    use mp_block::MadaraBlock;
    use mp_chain_config::StarknetVersion;
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Test the `l2_verify_and_apply_task` function.
    ///
    ///
    /// This test verifies the behavior of the `l2_verify_and_apply_task` by simulating
    /// a block verification and application process.
    ///
    /// # Test Steps
    /// 1. Initialize the backend and necessary components.
    /// 2. Create a mock block.
    /// 3. Spawn the `l2_verify_and_apply_task` in a new thread.
    /// 4. Send the mock block for verification and application.
    /// 5. Wait for the task to complete or for a timeout to occur.
    /// 6. Verify that the block has been correctly applied to the backend.
    ///
    /// # Panics
    /// - If the task fails or if the waiting timeout is exceeded.
    /// - If the block is not correctly applied to the backend.
    #[rstest]
    #[tokio::test]
    async fn test_l2_verify_and_apply_task(test_setup: Arc<MadaraBackend>) {
        let backend = test_setup;
        let (block_conv_sender, block_conv_receiver) = mpsc::channel(100);
        let block_import = Arc::new(BlockImporter::new(backend.clone(), None).unwrap());
        let validation = BlockValidationContext::new(backend.chain_config().chain_id.clone());
        let telemetry = Arc::new(TelemetryService::new(vec![]).unwrap().new_handle());

        let mock_block = create_dummy_unverified_full_block();

        let task_handle = tokio::spawn(l2_verify_and_apply_task(
            backend.clone(),
            ServiceContext::new_for_testing(),
            L2VerifyApplyConfig {
                block_import: block_import.clone(),
                backup_every_n_blocks: Some(1),
                flush_every_n_blocks: 1,
                flush_every_n_seconds: 10,
                stop_on_sync: false,
                telemetry,
                validation: validation.clone(),
                block_conv_receiver,
            },
        ));

        let mock_pre_validated_block = block_import.pre_validate(mock_block, validation.clone()).await.unwrap();
        block_conv_sender.send(mock_pre_validated_block).await.unwrap();

        drop(block_conv_sender);

        match tokio::time::timeout(std::time::Duration::from_secs(120), task_handle).await {
            Ok(Ok(_)) => (),
            Ok(Err(e)) => panic!("Task failed: {:?}", e),
            Err(_) => panic!("Timeout reached while waiting for task completion"),
        }

        let applied_block = backend.get_block(&DbBlockId::Number(0)).unwrap();
        assert!(applied_block.is_some(), "The block was not applied correctly");
        let applied_block = MadaraBlock::try_from(applied_block.unwrap()).unwrap();

        assert_eq!(applied_block.info.header.block_number, 0, "Block number does not match");
        assert_eq!(applied_block.info.header.block_timestamp.0, 0, "Block timestamp does not match");
        assert_eq!(applied_block.info.header.parent_block_hash, Felt::ZERO, "Parent block hash does not match");
        assert!(applied_block.inner.transactions.is_empty(), "Block should not contain any transactions");
        assert_eq!(
            applied_block.info.header.protocol_version,
            StarknetVersion::default(),
            "Protocol version does not match"
        );
        assert_eq!(applied_block.info.header.sequencer_address, Felt::ZERO, "Sequencer address does not match");
        assert_eq!(applied_block.info.header.l1_gas_price.eth_l1_gas_price, 0, "L1 gas price (ETH) does not match");
        assert_eq!(applied_block.info.header.l1_gas_price.strk_l1_gas_price, 0, "L1 gas price (STRK) does not match");
        assert_eq!(applied_block.info.header.l1_da_mode, L1DataAvailabilityMode::Blob, "L1 DA mode does not match");
    }

    /// Test the `l2_block_conversion_task` function.
    ///
    /// Steps:
    /// 1. Initialize necessary components.
    /// 2. Create a mock block.
    /// 3. Send the mock block to updates_sender
    /// 4. Call the `l2_block_conversion_task` function with the mock data.
    /// 5. Verify the results and ensure the function behaves as expected.
    #[rstest]
    #[tokio::test]
    async fn test_l2_block_conversion_task(test_setup: Arc<MadaraBackend>) {
        let backend = test_setup;
        let (updates_sender, updates_receiver) = mpsc::channel(100);
        let (output_sender, mut output_receiver) = mpsc::channel(100);
        let block_import = Arc::new(BlockImporter::new(backend.clone(), None).unwrap());
        let validation = BlockValidationContext::new(backend.chain_config().chain_id.clone());

        let mock_block = create_dummy_unverified_full_block();

        updates_sender.send(mock_block).await.unwrap();

        let task_handle = tokio::spawn(l2_block_conversion_task(
            updates_receiver,
            output_sender,
            block_import,
            validation,
            ServiceContext::new_for_testing(),
        ));

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), output_receiver.recv()).await;
        match result {
            Ok(Some(b)) => {
                assert_eq!(b.unverified_block_number, Some(0), "Block number does not match");
            }
            Ok(None) => panic!("Channel closed without receiving a result"),
            Err(_) => panic!("Timeout reached while waiting for result"),
        }

        // Close the updates_sender channel to allow the task to complete
        drop(updates_sender);

        match tokio::time::timeout(std::time::Duration::from_secs(5), task_handle).await {
            Ok(Ok(_)) => (),
            Ok(Err(e)) => panic!("Task failed: {:?}", e),
            Err(_) => panic!("Timeout reached while waiting for task completion"),
        }
    }

    /// Test the `l2_pending_block_task` function.
    ///
    /// This test function verifies the behavior of the `l2_pending_block_task`.
    /// It simulates the necessary environment and checks that the task executes correctly
    /// within a specified timeout.
    ///
    /// # Test Steps
    /// 1. Initialize the backend and test context.
    /// 2. Create a `BlockImporter` and a `BlockValidationContext`.
    /// 3. Spawn the `l2_pending_block_task` in a new thread.
    /// 4. Simulate the "once_caught_up" signal.
    /// 5. Wait for the task to complete or for a timeout to occur.
    ///
    /// # Panics
    /// - If the task fails or if the waiting timeout is exceeded.
    #[rstest]
    #[tokio::test]
    async fn test_l2_pending_block_task(test_setup: Arc<MadaraBackend>) {
        let backend = test_setup;
        let ctx = TestContext::new(backend.clone());
        let block_import = Arc::new(BlockImporter::new(backend.clone(), None).unwrap());
        let validation = BlockValidationContext::new(backend.chain_config().chain_id.clone());

        let task_handle = tokio::spawn(l2_pending_block_task(
            backend.clone(),
            ctx.provider.clone(),
            ServiceContext::new_for_testing(),
            L2PendingBlockConfig {
                block_import: block_import.clone(),
                once_caught_up_receiver: ctx.once_caught_up_receiver,
                pending_block_poll_interval: std::time::Duration::from_secs(5),
                validation: validation.clone(),
            },
        ));

        // Simulate the "once_caught_up" signal
        ctx.once_caught_up_sender.send(()).unwrap();

        // Wait for the task to complete
        match tokio::time::timeout(std::time::Duration::from_secs(120), task_handle).await {
            Ok(Ok(_)) => (),
            Ok(Err(e)) => panic!("Task failed: {:?}", e),
            Err(_) => panic!("Timeout reached while waiting for task completion"),
        }
    }
}
