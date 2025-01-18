use mp_utils::graceful_shutdown;

use crate::p2p::P2pPipelineArguments;

/// Pipeline order:
/// ```plaintext
///  ┌───────┐    ┌───────────┐     ┌───────┐           
///  │headers├─┬─►│state_diffs├──┬─►│classes│           
///  └───────┘ │  └───────────┘  │  └───────┘           
///            │                 │                      
///            │  ┌────────────┐ │  ┌──────────────────┐
///            ├─►│tx, receipts│ └─►│update_global_trie│
///            │  └────────────┘    └──────────────────┘
///            │                                        
///            │  ┌──────┐                              
///            └─►│events│                              
///               └──────┘                              
/// ```
/// State diffs, transactions with receipt, and events are checked against their corresponding commitments
/// in the header.
/// However, there is no commitment for classes: we instead check them against the state diff.
/// The update_global_trie step is a pipeline that only has a sequential part. It is separated from the state_diff step
/// because we want to apply big batches of state-diffs all at once in db, as an optimization. We want the batch size related
/// to that to be different from the batch size used to get state_diffs from p2p.
///
/// Headers are checked using the consensus signatures for forward sync.
///
/// ## Backwards mode
///
/// This is not implemented yet; however there will be a mode where we check the blocks
/// instead by going backwards from the latest block_hash verified on L1, and matching each earlier block with
/// the parent_hash of its successor. This mode won't need to check block signatures, but it can only help us
/// catch up with the latest block on L1. Syncing will switch in forward mode after that point, and consensus signatures
/// will be checked from that point on.
/// Until snap-sync is a thing, we also have to sync all state diffs in forward more.

pub struct ForwardSyncConfig {
    pub headers_parallelization: usize,
    pub headers_batch_size: usize,
    pub transactions_parallelization: usize,
    pub transactions_batch_size: usize,
    pub state_diffs_parallelization: usize,
    pub state_diffs_batch_size: usize,
    pub events_parallelization: usize,
    pub events_batch_size: usize,
    pub classes_parallelization: usize,
    pub classes_batch_size: usize,
    pub apply_state_parallelization: usize,
    pub apply_state_batch_size: usize,
}

impl Default for ForwardSyncConfig {
    fn default() -> Self {
        Self {
            headers_parallelization: 2,
            headers_batch_size: 4,
            transactions_parallelization: 3,
            transactions_batch_size: 2,
            state_diffs_parallelization: 3,
            state_diffs_batch_size: 2,
            events_parallelization: 2,
            events_batch_size: 2,
            classes_parallelization: 2,
            classes_batch_size: 2,
            apply_state_parallelization: 2,
            apply_state_batch_size: 2,
        }
    }
}

/// Events pipeline is currently always done after tx and receipts for now.
/// TODO: fix that when the db supports saving events separately.
pub async fn forward_sync(args: P2pPipelineArguments, config: ForwardSyncConfig) -> anyhow::Result<()> {
    let mut headers_pipeline =
        crate::headers::headers_pipeline(args.clone(), config.headers_parallelization, config.headers_batch_size);
    let mut transactions_pipeline = crate::transactions::transactions_pipeline(
        args.clone(),
        config.transactions_parallelization,
        config.transactions_batch_size,
    );
    let mut state_diffs_pipeline = crate::state_diffs::state_diffs_pipeline(
        args.clone(),
        config.state_diffs_parallelization,
        config.state_diffs_batch_size,
    );
    let mut classes_pipeline =
        crate::classes::classes_pipeline(args.clone(), config.classes_parallelization, config.classes_batch_size);
    let mut events_pipeline =
        crate::events::events_pipeline(args.clone(), config.events_parallelization, config.events_batch_size);
    let mut apply_state_pipeline = crate::apply_state::apply_state_pipeline(
        args.backend.clone(),
        args.importer.clone(),
        config.apply_state_parallelization,
        config.apply_state_batch_size,
    );

    loop {
        while headers_pipeline.can_schedule_more() {
            // todo follow until l1 head
            headers_pipeline.push(std::iter::once(()))
        }

        tokio::select! {
            _ = graceful_shutdown() => break,

            Some(res) = headers_pipeline.next(), if transactions_pipeline.can_schedule_more() && state_diffs_pipeline.can_schedule_more() => {
                let (_range, headers) = res?;
                transactions_pipeline.push(headers.iter().cloned());
                state_diffs_pipeline.push(headers);
            }
            Some(res) = transactions_pipeline.next(), if events_pipeline.can_schedule_more() => {
                let (_range, headers) = res?;
                events_pipeline.push(headers);
            }
            Some(res) = events_pipeline.next() => {
                res?;
            }
            Some(res) = state_diffs_pipeline.next(), if classes_pipeline.can_schedule_more() => {
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
