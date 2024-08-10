use crate::{BlockFetcher, BlockImportService, PipelineSettings, Validation};
use anyhow::Context;
use std::{ops::Range, sync::Arc};
use tokio::{sync::{mpsc, oneshot}, task::JoinSet};

use self::fetch_convert_task::fetch_convert_task;
mod fetch_convert_task;
mod verify_apply_task;

impl BlockImportService {
    fn block_range(&self, pipeline_settings: &PipelineSettings) -> anyhow::Result<Range<u64>> {
        let mut range_start = self
            .backend
            .get_latest_block_n()
            .context("Getting latest block number")?
            // latest block + 1, or fetch genesis if no block in db
            .map(|block| block + 1)
            .unwrap_or(0);

        if let Some(n) = pipeline_settings.first_block {
            range_start = u64::max(range_start, n);
        }

        let mut range_end = u64::MAX; // range not inclusive
        if let Some(n) = pipeline_settings.n_blocks_to_sync {
            range_end = u64::min(range_end, range_start + n);
        }
        if let Some(n) = pipeline_settings.end_at {
            range_end = u64::min(range_end, n + 1);
        }
        Ok(range_start..range_end)
    }

    /// Run the block pipeline, using a [`BlockFetcher`] for fetching blocks.
    pub async fn drive_pipeline(
        &self,
        validation: &Validation,
        block_fetcher: Arc<dyn BlockFetcher>,
        pipeline_settings: &PipelineSettings,
    ) -> anyhow::Result<()> {
        // let block_range = self.block_range(pipeline_settings)?;

        // let (stream_sender, stream_receiver) = mpsc::channel(pipeline_settings.channel_size);
        // let (once_caught_up_cb_sender, once_caught_up_cb_receiver) = oneshot::channel();

        // let mut join_set = JoinSet::new();
        // join_set.spawn(fetch_convert_task(
        //     validation,
        //     pipeline_settings,
        //     &block_fetcher,
        //     block_range,
        //     output,
        //     once_caught_up_callback,
        // ));

        Ok(())
    }
}
