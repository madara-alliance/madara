use std::time::Duration;
use std::{num::NonZeroUsize, sync::Arc};

use futures::prelude::*;
use mc_block_import::UnverifiedFullBlock;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mc_rpc::versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Client;
use mp_gateway::error::{SequencerError, StarknetError, StarknetErrorCode};
use mp_utils::service::ServiceContext;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::fetch::fetchers::fetch_block_and_updates;

use self::fetchers::WarpUpdateConfig;

pub mod fetchers;

pub struct L2FetchConfig {
    pub first_block: u64,
    pub fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
    pub once_caught_up_sender: oneshot::Sender<()>,
    pub sync_polling_interval: Option<Duration>,
    pub n_blocks_to_sync: Option<u64>,
    pub stop_on_sync: bool,
    pub sync_parallelism: usize,
    pub warp_update: Option<WarpUpdateConfig>,
}

pub async fn l2_fetch_task(
    backend: Arc<MadaraBackend>,
    provider: Arc<GatewayProvider>,
    mut ctx: ServiceContext,
    mut config: L2FetchConfig,
) -> anyhow::Result<()> {
    // First, catch up with the chain
    let L2FetchConfig { first_block, ref warp_update, .. } = config;

    if let Some(WarpUpdateConfig {
        warp_update_port_rpc,
        warp_update_port_fgw,
        warp_update_shutdown_sender,
        warp_update_shutdown_receiver,
        deferred_service_start,
        deferred_service_stop,
    }) = warp_update
    {
        let client = jsonrpsee::http_client::HttpClientBuilder::default()
            .build(format!("http://localhost:{warp_update_port_rpc}"))
            .expect("Building client");

        if client.ping().await.is_err() {
            tracing::error!("❗ Failed to connect to warp update sender on http://localhost:{warp_update_port_rpc}");
            ctx.cancel_global();
            return Ok(());
        }

        let provider = Arc::new(GatewayProvider::new(
            Url::parse(&format!("http://localhost:{warp_update_port_fgw}/gateway/"))
                .expect("Failed to parse warp update sender gateway url. This should not fail in prod"),
            Url::parse(&format!("http://localhost:{warp_update_port_fgw}/feeder_gateway/"))
                .expect("Failed to parse warp update sender feeder gateway url. This should not fail in prod"),
        ));

        let save = config.sync_parallelism;
        let available_parallelism = std::thread::available_parallelism()
            .unwrap_or(NonZeroUsize::new(1usize).expect("1 should always be in usize bound"));
        config.sync_parallelism = Into::<usize>::into(available_parallelism) * 2;

        let next_block = match sync_blocks(backend.as_ref(), &provider, &mut ctx, &config).await? {
            SyncStatus::Full(next_block) => next_block,
            SyncStatus::UpTo(next_block) => next_block,
        };

        if *warp_update_shutdown_sender {
            if client.shutdown().await.is_err() {
                tracing::error!("❗ Failed to shutdown warp update sender");
                ctx.cancel_global();
                return Ok(());
            }

            for svc_id in deferred_service_stop {
                ctx.service_remove(*svc_id);
            }

            for svc_id in deferred_service_start {
                ctx.service_add(*svc_id);
            }
        }

        if *warp_update_shutdown_receiver {
            return anyhow::Ok(());
        }

        config.n_blocks_to_sync = config.n_blocks_to_sync.map(|n| n - (next_block - first_block));
        config.first_block = next_block;
        config.sync_parallelism = save;
    }

    let mut next_block = match sync_blocks(backend.as_ref(), &provider, &mut ctx, &config).await? {
        SyncStatus::Full(next_block) => {
            tracing::info!("🥳 The sync process has caught up with the tip of the chain");
            next_block
        }
        SyncStatus::UpTo(next_block) => next_block,
    };

    if config.stop_on_sync {
        return anyhow::Ok(());
    }

    let L2FetchConfig { fetch_stream_sender, once_caught_up_sender, sync_polling_interval, stop_on_sync, .. } = config;

    // We do not call cancellation here as we still want the blocks to be stored
    if stop_on_sync {
        return anyhow::Ok(());
    }

    // TODO: replace this with a tokio::sync::Notify
    let _ = once_caught_up_sender.send(());

    if let Some(sync_polling_interval) = sync_polling_interval {
        // Polling

        let mut interval = tokio::time::interval(sync_polling_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while ctx.run_until_cancelled(interval.tick()).await.is_some() {
            // It is possible the chain produces multiple blocks in the span of
            // a single loop iteration, so we keep fetching until we reach the
            // tip again.
            let chain_id = &backend.chain_config().chain_id;
            let fetch = |next_block: u64| fetch_block_and_updates(chain_id, next_block, &provider);

            while let Some(block) = ctx.run_until_cancelled(fetch(next_block)).await {
                match block {
                    Err(FetchError::Sequencer(SequencerError::StarknetError(StarknetError {
                        code: StarknetErrorCode::BlockNotFound,
                        ..
                    }))) => {
                        break;
                    }
                    Err(e) => {
                        tracing::debug!("Failed to poll latest block: {e}");
                        return Err(e.into());
                    }
                    Ok(unverified_block) => {
                        if fetch_stream_sender.send(unverified_block).await.is_err() {
                            // stream closed
                            break;
                        }
                    }
                }

                next_block += 1;
            }
        }
    }

    anyhow::Ok(())
}

/// Whether a chain has been caught up to the tip or only a certain block number
///
/// This is mostly relevant in the context of the `--n-blocks-to-sync` cli
/// argument which states that a node might stop synchronizing before it has
/// caught up with the tip of the chain.
enum SyncStatus {
    Full(u64),
    UpTo(u64),
}

/// Sync blocks in parallel from a [GatewayProvider]
///
/// This function is called during warp update as well as l2 catch up to sync
/// to the tip of a chain. In the case of warp update, this is the tip of the
/// chain provided by the warp update sender. In the case of l2 sync, this is
/// the tip of the entire chain (mainnet, sepolia, devnet).
///
/// This function _is not_ called after the chain has been synced as this has
/// a different fetch logic which does not fetch block in parallel.
///
/// Fetch config, including number of blocks to fetch and fetch parallelism,
/// is defined in [L2FetchConfig].
async fn sync_blocks(
    backend: &MadaraBackend,
    provider: &Arc<GatewayProvider>,
    ctx: &mut ServiceContext,
    config: &L2FetchConfig,
) -> anyhow::Result<SyncStatus> {
    let L2FetchConfig { first_block, fetch_stream_sender, n_blocks_to_sync, sync_parallelism, .. } = config;

    tracing::info!("######### n blocks to sync: {:?}", n_blocks_to_sync);
    
    // Create a bounded range of block numbers to fetch
    let block_range = match n_blocks_to_sync {
        Some(n) => (*first_block..(*n)),
        None => (*first_block..u64::MAX), // Unbounded if no limit specified
    };
    
    tracing::info!("######### block range: {:?}", block_range);
    // Fetch blocks and updates in parallel one time before looping
    let fetch_stream = block_range.map(|block_n| {
        let provider = Arc::clone(provider);
        let chain_id = &backend.chain_config().chain_id;
        async move { (block_n, fetch_block_and_updates(chain_id, block_n, &provider).await) }
    });

    // Have `sync_parallelism` fetches in parallel at once, using futures Buffered
    let mut next_block = *first_block;
    let mut fetch_stream = stream::iter(fetch_stream).buffered(*sync_parallelism);

    while let Some(next) = ctx.run_until_cancelled(fetch_stream.next()).await {
        let Some((block_n, val)) = next else {
            return anyhow::Ok(SyncStatus::UpTo(next_block));
        };

        match val {
            Err(FetchError::Sequencer(SequencerError::StarknetError(StarknetError {
                code: StarknetErrorCode::BlockNotFound,
                ..
            }))) => {
                return anyhow::Ok(SyncStatus::Full(next_block));
            }
            val => {
                if fetch_stream_sender.send(val?).await.is_err() {
                    // join error
                    return anyhow::Ok(SyncStatus::UpTo(next_block));
                }
            }
        }

        next_block = block_n + 1;
    }

    anyhow::Ok(SyncStatus::UpTo(next_block))
}

#[derive(thiserror::Error, Debug)]
pub enum FetchError {
    #[error(transparent)]
    Sequencer(#[from] SequencerError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[cfg(test)]
mod test_l2_fetch_task {
    use super::*;
    use crate::tests::utils::gateway::{test_setup, TestContext};
    use rstest::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// Test the comprehensive functionality of the l2_fetch_task.
    ///
    /// This test verifies that:
    /// 1. The task can fetch an initial set of blocks.
    /// 2. The task sends the "caught up" signal after the initial fetch.
    /// 3. The task continues to poll for new blocks at the specified interval.
    /// 4. The task can fetch new blocks that appear after the initial sync.
    #[rstest]
    #[tokio::test]
    async fn test_l2_fetch_task_comprehensive(test_setup: Arc<MadaraBackend>) {
        let mut ctx = TestContext::new(test_setup);

        for block_number in 0..8 {
            ctx.mock_block(block_number);
        }

        ctx.mock_class_hash(m_cairo_test_contracts::TEST_CONTRACT_SIERRA);
        ctx.mock_signature();

        let polling_interval = Duration::from_millis(100);
        let task = tokio::spawn({
            let backend = Arc::clone(&ctx.backend);
            let provider = Arc::clone(&ctx.provider);
            let fetch_stream_sender = ctx.fetch_stream_sender.clone();
            let once_caught_up_sender = ctx.once_caught_up_sender;
            async move {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    l2_fetch_task(
                        backend,
                        provider,
                        ServiceContext::new_for_testing(),
                        L2FetchConfig {
                            first_block: 0,
                            fetch_stream_sender,
                            once_caught_up_sender,
                            sync_polling_interval: Some(polling_interval),
                            n_blocks_to_sync: Some(5),
                            stop_on_sync: false,
                            sync_parallelism: 10,
                            warp_update: None,
                        },
                    ),
                )
                .await
            }
        });

        for expected_block_number in 0..5 {
            match tokio::time::timeout(Duration::from_secs(1), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                }
                Ok(None) => panic!("Channel closed unexpectedly"),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        match tokio::time::timeout(Duration::from_secs(1), ctx.once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        tokio::time::sleep(Duration::from_millis(300)).await;

        for expected_block_number in 5..8 {
            match tokio::time::timeout(Duration::from_secs(1), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                }
                Ok(None) => panic!("Channel closed unexpectedly"),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        assert!(!task.is_finished());

        task.abort();
    }
}
