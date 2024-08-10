use dp_utils::{channel_wait_or_graceful_shutdown, wait_or_graceful_shutdown};
use futures::{stream, StreamExt};
use tokio::sync::{mpsc, oneshot};

use crate::{
    pre_validate, BlockFetcher, BlockFetcherError, BlockImportError, PipelineSettings, PreValidatedBlock, Validation,
};

async fn fetch_and_convert(
    block_fetcher: &dyn BlockFetcher,
    block_n: u64,
    validation: &Validation,
) -> Result<Option<PreValidatedBlock>, BlockImportError> {
    let block = match block_fetcher.fetch_block(block_n).await {
        Err(BlockFetcherError::BlockNotFound) => return Ok(None),
        Err(BlockFetcherError::Custom(err)) => return Err(BlockImportError::InternalFetching(err)),
        Ok(block) => block,
    };

    let block = pre_validate(block, validation).await?;
    Ok(Some(block))
}

pub async fn fetch_convert_task(
    validation: &Validation,
    pipeline_settings: &PipelineSettings,
    block_fetcher: &dyn BlockFetcher,
    block_range: impl IntoIterator<Item = u64>,
    output: mpsc::Sender<PreValidatedBlock>,
    once_caught_up_callback: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    // First, catch up with the chain

    let mut block_range = block_range.into_iter().peekable();

    let Some(mut next_block) = block_range.peek().copied() else { return Ok(()) };

    {
        let fetch_stream = block_range
            .into_iter()
            .map(|block_n| async move { (block_n, fetch_and_convert(block_fetcher, block_n, validation).await) });

        // Have 10 fetches in parallel at once, using futures Buffered
        // this takes an iterator of futures
        let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
        while let Some((block_n, val)) = channel_wait_or_graceful_shutdown(fetch_stream.next()).await {
            log::debug!("got {:?}", block_n);

            match val {
                Err(err) => {
                    anyhow::bail!("Error while importing block {block_n}: {:#}", err);
                }
                Ok(None) => {
                    log::info!("🥳 The sync process has caught up with the tip of the chain");
                    break;
                }
                Ok(Some(val)) => {
                    if output.send(val).await.is_err() {
                        // stream closed
                        break;
                    }
                }
            }

            next_block = block_n + 1;
        }
    };

    log::debug!("caught up with tip");
    let _ = once_caught_up_callback.send(());

    if let Some(sync_polling) = &pipeline_settings.polling {
        // Polling

        let mut interval = tokio::time::interval(sync_polling.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while wait_or_graceful_shutdown(interval.tick()).await.is_some() {
            loop {
                match fetch_and_convert(block_fetcher, next_block, validation).await? {
                    None => {
                        break;
                    }
                    Some(val) => {
                        if output.send(val).await.is_err() {
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
