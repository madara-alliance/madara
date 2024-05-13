use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use futures::prelude::*;
use starknet_core::types::StarknetError;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use tokio::sync::mpsc;

use self::fetchers::L2BlockAndUpdates;
use crate::fetch::fetchers::fetch_block_and_updates;
use crate::l2::L2SyncError;

pub mod fetchers;

pub async fn l2_fetch_task(
    first_block: u64,
    n_blocks_to_sync: Option<u64>,
    fetch_stream_sender: mpsc::Sender<L2BlockAndUpdates>,
    provider: Arc<SequencerGatewayProvider>,
    sync_polling_interval: Option<Duration>,
) -> anyhow::Result<()> {
    // First, catch up with the chain

    let mut next_block = first_block;

    {
        // Fetch blocks and updates in parallel one time before looping
        let fetch_stream = (first_block..).take(n_blocks_to_sync.unwrap_or(u64::MAX) as _).map(|block_n| {
            let provider = Arc::clone(&provider);
            async move { (block_n, fetch_block_and_updates(block_n, provider).await) }
        });

        // Have 10 fetches in parallel at once, using futures Buffered
        let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
        while let Some((block_n, val)) = pin!(fetch_stream.next()).await {
            log::debug!("got {:?}", block_n);

            match val {
                Err(L2SyncError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                    break;
                }
                val => fetch_stream_sender.send(val?).await.expect("reciever task is closed"),
            }

            next_block = block_n + 1;
        }
    };

    log::info!("🥳🥳🥳 The sync process caught up with the tip of the chain.");

    if let Some(sync_polling_interval) = sync_polling_interval {
        // Polling

        let mut interval = tokio::time::interval(sync_polling_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;

            loop {
                match fetch_block_and_updates(next_block, Arc::clone(&provider)).await {
                    Err(L2SyncError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                        break;
                    }
                    val => fetch_stream_sender.send(val?).await.expect("reciever task is closed"),
                }

                next_block += 1;
            }
        }
    }
    Ok(())
}
