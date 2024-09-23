use std::sync::Arc;
use std::time::Duration;

use futures::prelude::*;
use mc_block_import::UnverifiedFullBlock;
use mc_db::MadaraBackend;
use mp_utils::{channel_wait_or_graceful_shutdown, wait_or_graceful_shutdown};
use starknet_core::types::StarknetError;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use tokio::sync::{mpsc, oneshot};

use crate::fetch::fetchers::fetch_block_and_updates;

pub mod fetchers;

#[allow(clippy::too_many_arguments)]
pub async fn l2_fetch_task(
    backend: Arc<MadaraBackend>,
    first_block: u64,
    n_blocks_to_sync: Option<u64>,
    fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
    provider: Arc<SequencerGatewayProvider>,
    sync_polling_interval: Option<Duration>,
    once_caught_up_callback: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    // First, catch up with the chain
    let backend = &backend;

    let mut next_block = first_block;

    {
        // Fetch blocks and updates in parallel one time before looping
        let fetch_stream = (first_block..).take(n_blocks_to_sync.unwrap_or(u64::MAX) as _).map(|block_n| {
            let provider = Arc::clone(&provider);
            async move { (block_n, fetch_block_and_updates(&backend.chain_config().chain_id, block_n, &provider).await) }
        });

        // Have 10 fetches in parallel at once, using futures Buffered
        let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
        while let Some((block_n, val)) = channel_wait_or_graceful_shutdown(fetch_stream.next()).await {
            match val {
                Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                    log::info!("🥳 The sync process has caught up with the tip of the chain");
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

    let _ = once_caught_up_callback.send(());

    if let Some(sync_polling_interval) = sync_polling_interval {
        // Polling

        let mut interval = tokio::time::interval(sync_polling_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while wait_or_graceful_shutdown(interval.tick()).await.is_some() {
            loop {
                match fetch_block_and_updates(&backend.chain_config().chain_id, next_block, &provider).await {
                    Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
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
    Provider(#[from] ProviderError),
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

        ctx.mock_class_hash("cairo/target/dev/madara_contracts_TestContract.contract_class.json");

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
                        0,
                        Some(5),
                        fetch_stream_sender,
                        provider,
                        Some(polling_interval),
                        once_caught_up_sender,
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
