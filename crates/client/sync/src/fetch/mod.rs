use std::sync::Arc;
use std::time::Duration;

use futures::prelude::*;
use mc_block_import::UnverifiedFullBlock;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mc_rpc::versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Client;
use mp_gateway::error::{SequencerError, StarknetError, StarknetErrorCode};
use mp_utils::{channel_wait_or_graceful_shutdown, service::ServiceContext, wait_or_graceful_shutdown};
use tokio::sync::{mpsc, oneshot};

use crate::fetch::fetchers::fetch_block_and_updates;

pub mod fetchers;

pub struct L2FetchConfig {
    pub first_block: u64,
    pub fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
    pub once_caught_up_sender: oneshot::Sender<()>,
    pub sync_polling_interval: Option<Duration>,
    pub n_blocks_to_sync: Option<u64>,
    pub stop_on_sync: bool,
    pub sync_parallelism: u8,
    pub warp_update: bool,
}

pub async fn l2_fetch_task(
    backend: Arc<MadaraBackend>,
    provider: Arc<GatewayProvider>,
    ctx: ServiceContext,
    config: L2FetchConfig,
) -> anyhow::Result<()> {
    // First, catch up with the chain
    let backend = &backend;

    let L2FetchConfig {
        first_block,
        fetch_stream_sender,
        once_caught_up_sender: once_caught_up_callback,
        sync_polling_interval,
        n_blocks_to_sync,
        stop_on_sync,
        sync_parallelism,
        warp_update,
    } = config;

    if warp_update {
        let ping = jsonrpsee::http_client::HttpClientBuilder::default()
            .build("http://localhost:9943")
            .expect("Building client")
            .ping()
            .await;

        if ping.is_err() {
            tracing::error!("❗ Failed to connect to warp update sender on http://localhost:9943");
            ctx.cancel_global();
        }
    }

    let mut next_block = first_block;

    {
        // Fetch blocks and updates in parallel one time before looping
        let fetch_stream = (first_block..).take(n_blocks_to_sync.unwrap_or(u64::MAX) as _).map(|block_n| {
            let provider = Arc::clone(&provider);
            let ctx = ctx.branch();
            async move {
                (block_n, fetch_block_and_updates(&backend.chain_config().chain_id, block_n, &provider, &ctx).await)
            }
        });

        // Have 10 fetches in parallel at once, using futures Buffered
        let mut fetch_stream = stream::iter(fetch_stream).buffered(sync_parallelism as usize);
        while let Some((block_n, val)) = channel_wait_or_graceful_shutdown(fetch_stream.next(), &ctx).await {
            match val {
                Err(FetchError::Sequencer(SequencerError::StarknetError(StarknetError {
                    code: StarknetErrorCode::BlockNotFound,
                    ..
                }))) => {
                    tracing::info!("🥳 The sync process has caught up with the tip of the chain");

                    if warp_update {
                        let shutdown = jsonrpsee::http_client::HttpClientBuilder::default()
                            .build("http://localhost:9943")
                            .expect("Building client")
                            .shutdown()
                            .await;

                        if shutdown.is_err() {
                            tracing::error!("❗ Failed to shutdown warp update sender");
                        }
                    }

                    break;
                }
                val => {
                    if fetch_stream_sender.send(val?).await.is_err() {
                        // join error
                        break;
                    }
                }
            }

            next_block = block_n + 1;
        }
    };

    // We do not call cancellation here as we still want the block to be stored
    if stop_on_sync {
        return anyhow::Ok(());
    }

    let _ = once_caught_up_callback.send(());

    if let Some(sync_polling_interval) = sync_polling_interval {
        // Polling

        let mut interval = tokio::time::interval(sync_polling_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while wait_or_graceful_shutdown(interval.tick(), &ctx).await.is_some() {
            loop {
                match fetch_block_and_updates(&backend.chain_config().chain_id, next_block, &provider, &ctx).await {
                    Err(FetchError::Sequencer(SequencerError::StarknetError(StarknetError {
                        code: StarknetErrorCode::BlockNotFound,
                        ..
                    }))) => {
                        break;
                    }
                    val => {
                        if fetch_stream_sender.send(val?).await.is_err() {
                            // stream closed
                            break;
                        }
                    }
                }

                next_block += 1;
            }
        }
    }
    Ok(())
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
                        tokio_util::sync::CancellationToken::new(),
                        L2FetchConfig {
                            first_block: 0,
                            fetch_stream_sender,
                            once_caught_up_sender,
                            sync_polling_interval: Some(polling_interval),
                            n_blocks_to_sync: Some(5),
                            stop_on_sync: false,
                            sync_parallelism: 10,
                            warp_update: false,
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
