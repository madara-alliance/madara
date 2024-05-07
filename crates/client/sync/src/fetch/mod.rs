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
    fetch_stream_sender: mpsc::Sender<L2BlockAndUpdates>,
    provider: Arc<SequencerGatewayProvider>,
    pending_polling_interval: Duration,
) -> Result<(), L2SyncError> {
    // First, catch up with the chain

    let mut last_block = {
        // Fetch blocks and updates in parallel one time before looping
        let fetch_stream = (first_block..).map(|block_n| {
            let provider = Arc::clone(&provider);
            async move { (block_n, fetch_block_and_updates(block_n, provider).await) }
        });

        // Have 10 fetches in parallel at once, using futures Buffered
        let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
        loop {
            let (block_n, val) = pin!(fetch_stream.next()).await.unwrap(); // UNWRAP: stream is infinite
            log::debug!("got {:?}", block_n);

            match val {
                Err(L2SyncError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                    break block_n;
                }
                val => fetch_stream_sender.send(val?).await.expect("reciever task is closed"),
            }
        }
    };

    log::debug!("We caught up with the chain yay");

    // Polling

    let mut interval = tokio::time::interval(pending_polling_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;

        loop {
            match fetch_block_and_updates(last_block + 1, Arc::clone(&provider)).await {
                Err(L2SyncError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                    break;
                }
                val => fetch_stream_sender.send(val?).await.expect("reciever task is closed"),
            }

            last_block += 1;
        }
    }
}
