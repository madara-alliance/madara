use crate::{db_block_id::DbBlockId, MadaraBackend, MadaraStorageError};
use crate::{Column, DatabaseExt};
use futures::{stream, Stream};
use mp_block::MadaraBlockInfo;
use std::iter;
use std::{
    collections::VecDeque,
    num::NonZeroU64,
    ops::{Bound, RangeBounds},
    sync::Arc,
};
use tokio::sync::broadcast::{error::RecvError, Receiver};

/// Returns (inclusive start, optional limit).
/// When the start is unbounded, we start at 0.
fn resolve_range(range: impl RangeBounds<u64>) -> (u64, Option<u64>) {
    let start = match range.start_bound() {
        Bound::Included(start) => *start,
        Bound::Excluded(start) => match start.checked_add(1) {
            Some(start) => start,
            None => {
                // start is u64::max, excluded. Return an empty range.
                return (u64::MAX, Some(0));
            }
        },
        Bound::Unbounded => 0,
    };
    let limit = match range.end_bound() {
        Bound::Included(end) => Some(end.saturating_add(1).saturating_sub(start)),
        Bound::Excluded(end) => Some(end.saturating_sub(start)),
        Bound::Unbounded => None,
    };

    (start, limit)
}

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq)]
pub enum Direction {
    #[default]
    Forward,
    Backward,
}
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BlockStreamConfig {
    pub direction: Direction,
    /// Block number from which to start (inclusive).
    /// In the case of reverse iteration, if the block does not exist yet, iteration will start from the latest block in db.
    pub start: u64,
    pub step: NonZeroU64,
    pub limit: Option<u64>,
}

impl BlockStreamConfig {
    pub fn backward(mut self) -> Self {
        self.direction = Direction::Backward;
        self
    }

    pub fn forward(mut self) -> Self {
        self.direction = Direction::Forward;
        self
    }

    pub fn with_block_range(mut self, range: impl RangeBounds<u64>) -> Self {
        let (start, limit) = resolve_range(range);
        self.start = start;
        self.limit = limit;
        self
    }

    pub fn with_limit(mut self, limit: impl Into<Option<u64>>) -> Self {
        self.limit = limit.into();
        self
    }

    pub fn with_start(mut self, start: impl Into<u64>) -> Self {
        self.start = start.into();
        self
    }
}

impl Default for BlockStreamConfig {
    fn default() -> Self {
        Self { direction: Direction::Forward, start: 0, step: NonZeroU64::MIN, limit: None }
    }
}

impl MadaraBackend {
    pub fn block_info_iterator(
        self: &Arc<Self>,
        iteration: BlockStreamConfig,
    ) -> impl Iterator<Item = Result<MadaraBlockInfo, MadaraStorageError>> {
        // The important thing here is to avoid keeping iterators around in the iterator state,
        // as an iterator instance will pin the memtables.
        // To avoid that, we buffer a few blocks ahead to still benefit from the rocksdb iterators.
        // Note, none of that has been benchmarked.
        const BUFFER_SIZE: usize = 32;

        struct State {
            backend: Arc<MadaraBackend>,
            iteration: BlockStreamConfig,
            buf: VecDeque<MadaraBlockInfo>,
            next_block_n: Option<u64>,
            total_got: u64,
        }

        impl State {
            fn next_item(&mut self) -> Result<Option<MadaraBlockInfo>, MadaraStorageError> {
                if self.buf.is_empty() {
                    if self.iteration.limit.is_some_and(|limit| self.total_got >= limit) {
                        return Ok(None);
                    }

                    let Some(mut next_block_n) = self.next_block_n else {
                        return Ok(None);
                    };

                    let col = self.backend.db.get_column(Column::BlockNToBlockInfo);
                    let mut ite = self.backend.db.raw_iterator_cf(&col);

                    match self.iteration.direction {
                        Direction::Forward => ite.seek(next_block_n.to_be_bytes()),
                        Direction::Backward => ite.seek_for_prev(next_block_n.to_be_bytes()),
                    }
                    for _ in 0..BUFFER_SIZE {
                        // End condition when moving forward is the latest full block in db.
                        if self.iteration.direction == Direction::Forward
                            && self.backend.head_status().next_full_block() <= next_block_n
                        {
                            break;
                        }

                        let Some((k, v)) = ite.item() else {
                            ite.status()?; // bubble up error, or, we reached the end.
                            break;
                        };

                        let block_n =
                            u64::from_be_bytes(k.try_into().map_err(|_| MadaraStorageError::InvalidBlockNumber)?);

                        if self.iteration.direction == Direction::Backward {
                            // If we asked for a block that does not yet exist, we start from the highest block found instead.
                            next_block_n = u64::min(next_block_n, block_n);
                        }

                        let val = bincode::deserialize(v)?;

                        self.buf.push_back(val);
                        self.total_got += 1;

                        // update next_block_n
                        match self.iteration.direction {
                            Direction::Forward => {
                                self.next_block_n = next_block_n.checked_add(self.iteration.step.get());
                            }
                            Direction::Backward => {
                                self.next_block_n = next_block_n.checked_sub(self.iteration.step.get());
                            }
                        }
                        let Some(next) = self.next_block_n else { break };
                        next_block_n = next;

                        if self.iteration.limit.is_some_and(|limit| self.total_got >= limit) {
                            break;
                        }

                        if self.iteration.step.get() > 1 {
                            // seek here instead of next/prev
                            match self.iteration.direction {
                                Direction::Forward => ite.seek(next_block_n.to_be_bytes()),
                                Direction::Backward => ite.seek_for_prev(next_block_n.to_be_bytes()),
                            }
                        } else {
                            match self.iteration.direction {
                                Direction::Forward => ite.next(),
                                Direction::Backward => ite.prev(),
                            }
                        }
                    }
                }

                Ok(self.buf.pop_front())
            }
        }

        let mut state = State {
            backend: Arc::clone(self),
            buf: VecDeque::with_capacity(BUFFER_SIZE),
            next_block_n: Some(iteration.start),
            total_got: 0,
            iteration,
        };
        iter::from_fn(move || state.next_item().transpose())
    }

    /// This function will follow the tip of the chain when asked for Forward iteration, hence it is a Stream and not an Iterator.
    pub fn block_info_stream(
        self: &Arc<Self>,
        iteration: BlockStreamConfig,
    ) -> impl Stream<Item = Result<MadaraBlockInfo, MadaraStorageError>> {
        // So, this is a somewhat funny problem: by the time we return the blocks until the current latest_block in db,
        //  the database may actually have new blocks now!
        // Remember that we're returning a stream here, which means that the time between polls varies with the caller - and,
        //  in the use cases we're interested in (websocket/p2p) the time between polls varies depending on the speed of the
        //  connection with the client/peer that's calling the endpoint.
        // So! it may very well be the case that once we caught up with the latest block_n in db as we saw at the beginning
        //  of the call, and once we sent all the blocks that have been added within the time we sent all of those, we might
        //  still not have caught up with the latest block in db - because new blocks could have come by then.
        // This implementation solves this problem by checking the latest block in db in a loop and only once it really looks
        //  like we caught up with the db, we subscribe to the new blocks channel. But hold on a minute, this subscribe is
        //  done after getting the latest block number! There's a split second where it could have been possible to miss a
        //  block. Because of this rare case, there are two supplementary things to note: we *also* get the latest block_n
        //  *after* subscribing, so that we can check that we did not miss anything during subscription - and just in case,
        //  we also handle the case when the subscription returns a block that is futher into the future than the one we
        //  would expect.
        // All in all, this implementation tries its *very best* not to subscribe to the channel when it does not have to.
        // In addition, because rust does not have `yield` syntax (yet? I'm losing hope..) - this is implemented as a
        //  funky looking state machine. yay!

        // TODO: use db iterators to fill a VecDeque buffer (we don't want to hold a db iterator across an await point!)
        //   => reuse block_info_iterator logic
        // TODO: what should we do about reorgs?! i would assume we go back and rereturn the new blocks..?

        struct State {
            iteration: BlockStreamConfig,
            backend: Arc<MadaraBackend>,
            /// `None` here means we reached the end of iteration.
            next_to_return: Option<u64>,
            num_blocks_returned: u64,
            /// This is `+ 1` because we want to handle returning genesis. If the chain is empty (does not even have a genesis
            /// block), this field will be 0.
            latest_plus_one: Option<u64>,
            subscription: Option<Receiver<MadaraBlockInfo>>,
        }

        impl State {
            /// Get the `latest_plus_one` variable in `self`, populating it if it is empty.
            fn get_latest_plus_one(&mut self) -> Result<u64, MadaraStorageError> {
                let latest_plus_one = match self.latest_plus_one {
                    Some(n) => n,
                    None => {
                        self.backend.get_latest_block_n()?.map(|n| n.saturating_add(1)).unwrap_or(/* genesis */ 0)
                    }
                };
                self.latest_plus_one = Some(latest_plus_one);
                Ok(latest_plus_one)
            }

            async fn next_forward(&mut self) -> Result<Option<MadaraBlockInfo>, MadaraStorageError> {
                'retry: loop {
                    let Some(next_to_return) = self.next_to_return else { return Ok(None) };

                    // If we have a subscription, return blocks from it.
                    if let Some(subscription) = &mut self.subscription {
                        match subscription.recv().await {
                            // return this block
                            Ok(info) if info.header.block_number == next_to_return => {
                                self.next_to_return = next_to_return.checked_add(self.iteration.step.get());
                                return Ok(Some(info));
                            }
                            // skip this block
                            Ok(info) if info.header.block_number < next_to_return => continue 'retry,
                            // the channel returned a block number that we didn't expect. Treat that as if it lagged..?
                            Ok(_info) => self.subscription = None,
                            // If it lagged (buffer full), continue using db and we'll eventually resubscribe again once caught up :)
                            Err(RecvError::Lagged(_n_skipped_messages)) => self.subscription = None,
                            Err(RecvError::Closed) => return Ok(None),
                        }
                    }

                    // Or else, return blocks from the db.

                    if self.latest_plus_one.is_some_and(|latest_plus_one| latest_plus_one <= next_to_return) {
                        // new blocks may have arrived, get latest_block_n again
                        self.latest_plus_one = None
                    }

                    let latest_plus_one = self.get_latest_plus_one()?;

                    if latest_plus_one <= next_to_return {
                        // caught up with the db :)
                        self.subscription = Some(self.backend.subscribe_block_info());
                        // get latest_block_n again after subscribing, because it could have changed during subscribing
                        self.latest_plus_one = None;
                        self.get_latest_plus_one()?;
                        continue 'retry;
                    }

                    let block_info = &self.backend.get_block_info(&DbBlockId::Number(next_to_return))?.ok_or(
                        MadaraStorageError::InconsistentStorage("latest_block_n points to a non existent block".into()),
                    )?;
                    let block_info = block_info
                        .as_nonpending()
                        .ok_or(MadaraStorageError::InconsistentStorage("Closed block should not be pending".into()))?;

                    self.next_to_return = next_to_return.checked_add(self.iteration.step.get());
                    return Ok(Some(block_info.clone()));
                }
            }

            // Implement backward mode in another function.
            async fn next_backward(&mut self) -> Result<Option<MadaraBlockInfo>, MadaraStorageError> {
                // This makes sure we're starting from a block that actually exists. It bounds the `next_to_return` variable.
                if self.latest_plus_one.is_none() {
                    let Some(next_to_return) = self.next_to_return else { return Ok(None) };
                    let latest_block = self.get_latest_plus_one()?.checked_sub(1);
                    // If there are no blocks in db, this will set `next_to_return` to None.
                    self.next_to_return = latest_block.map(|latest_block| u64::min(latest_block, next_to_return))
                }

                let Some(next_to_return) = self.next_to_return else { return Ok(None) };

                let block_info = &self.backend.get_block_info(&DbBlockId::Number(next_to_return))?.ok_or(
                    MadaraStorageError::InconsistentStorage("latest_block_n points to a non existent block".into()),
                )?;
                let block_info = block_info
                    .as_nonpending()
                    .ok_or(MadaraStorageError::InconsistentStorage("Closed block should not be pending".into()))?;

                // The None here will stop the iteration once we passed genesis.
                self.next_to_return = next_to_return.checked_sub(self.iteration.step.get());
                Ok(Some(block_info.clone()))
            }

            async fn try_next(&mut self) -> Result<Option<MadaraBlockInfo>, MadaraStorageError> {
                if self.iteration.limit.is_some_and(|limit| self.num_blocks_returned >= limit) {
                    return Ok(None);
                }

                let ret = match self.iteration.direction {
                    Direction::Forward => self.next_forward().await?,
                    Direction::Backward => self.next_backward().await?,
                };

                if ret.is_some() {
                    self.num_blocks_returned = self.num_blocks_returned.saturating_add(1);
                }

                Ok(ret)
            }
        }

        stream::unfold(
            State {
                next_to_return: Some(iteration.start),
                iteration,
                num_blocks_returned: 0,
                latest_plus_one: None,
                backend: Arc::clone(self),
                subscription: None,
            },
            |mut s| async { s.try_next().await.transpose().map(|el| (el, s)) },
        )
    }
}

#[cfg(test)]
mod tests {
    //! To test:
    //! - [x] Simple iteration, everything in db.
    //! - [x] Simple iteration, db is empty.
    //! - [x] Simple iteration, everything in db. Start from a specific block.
    //! - [x] Simple iteration, everything in db. Start from a block that doesnt exist yet.
    //! - [x] More complex cases where blocks are added during iteration.
    //! - [x] Reverse iteration.
    //! - [x] Reverse iteration, db is empty.
    //! - [x] Reverse iteration: start from a specific block.
    //! - [x] Reverse: Start from a block that doesnt exist yet.
    //! - [x] Step iteration, forward.
    //! - [x] Step iteration, backward.
    //! - [x] Limit field.
    //! - [x] Limit field wait on channel.
    //! - [x] Limit field reverse iteration.

    use super::*;
    use mp_block::{Header, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
    use mp_chain_config::ChainConfig;
    use starknet_types_core::felt::Felt;
    use std::time::Duration;
    use stream::{StreamExt, TryStreamExt};
    use tokio::{pin, time::timeout};

    fn block_info(block_number: u64) -> MadaraBlockInfo {
        MadaraBlockInfo {
            header: Header { block_number, ..Default::default() },
            block_hash: Felt::from(block_number),
            tx_hashes: Default::default(),
        }
    }

    fn store_block(backend: &MadaraBackend, block_number: u64) {
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    inner: Default::default(),
                    info: MadaraMaybePendingBlockInfo::NotPending(block_info(block_number)),
                },
                Default::default(),
                Default::default(),
            )
            .unwrap();
    }

    #[rstest::fixture]
    fn empty_chain() -> Arc<MadaraBackend> {
        MadaraBackend::open_for_testing(ChainConfig::madara_test().into())
    }

    #[rstest::fixture]
    fn test_chain() -> Arc<MadaraBackend> {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        for block_number in 0..5 {
            store_block(&backend, block_number)
        }
        backend
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_simple(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig::default());
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(1)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());

        store_block(&test_chain, 5);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(5)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_empty_chain(empty_chain: Arc<MadaraBackend>) {
        let stream = empty_chain.block_info_stream(BlockStreamConfig::default());
        pin!(stream);
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());

        store_block(&empty_chain, 0);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_start_from_block(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig { start: 3, ..Default::default() });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());

        store_block(&test_chain, 5);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(5)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_start_from_not_yet_created(empty_chain: Arc<MadaraBackend>) {
        let stream = empty_chain.block_info_stream(BlockStreamConfig { start: 3, ..Default::default() });
        pin!(stream);

        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 0);
        store_block(&empty_chain, 1);
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 2);
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 3);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 4);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_concurrent(empty_chain: Arc<MadaraBackend>) {
        let stream = empty_chain.block_info_stream(BlockStreamConfig::default());
        pin!(stream);

        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 0);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 1);
        store_block(&empty_chain, 2);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(1)));
        store_block(&empty_chain, 3);
        store_block(&empty_chain, 4);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&empty_chain, 5);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(5)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_backward(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig {
            direction: Direction::Backward,
            start: 3,
            ..Default::default()
        });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(1)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_backward_empty(empty_chain: Arc<MadaraBackend>) {
        let stream = empty_chain.block_info_stream(BlockStreamConfig {
            direction: Direction::Backward,
            start: 0,
            ..Default::default()
        });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_backward_start_from_not_yet_created(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig {
            direction: Direction::Backward,
            start: 10,
            ..Default::default()
        });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(1)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_step(test_chain: Arc<MadaraBackend>) {
        let stream =
            test_chain.block_info_stream(BlockStreamConfig { step: 2.try_into().unwrap(), ..Default::default() });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());

        store_block(&test_chain, 5);
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());

        store_block(&test_chain, 6);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(6)));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_step_backward(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig {
            direction: Direction::Backward,
            step: 2.try_into().unwrap(),
            start: 4,
            ..Default::default()
        });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_limit(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig { limit: Some(3), ..Default::default() });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(0)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(1)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_limit2(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig { limit: Some(3), start: 4, ..Default::default() });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&test_chain, 5);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(5)));
        assert!(timeout(Duration::from_millis(50), stream.next()).await.is_err());
        store_block(&test_chain, 6);
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(6)));
        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_limit_backward(test_chain: Arc<MadaraBackend>) {
        let stream = test_chain.block_info_stream(BlockStreamConfig {
            direction: Direction::Backward,
            limit: Some(3),
            start: 5,
            ..Default::default()
        });
        pin!(stream);

        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(4)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(3)));
        assert_eq!(stream.try_next().await.unwrap(), Some(block_info(2)));
        assert_eq!(stream.try_next().await.unwrap(), None);
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)]
    fn test_resolve_range() {
        assert_eq!(resolve_range(0..), (0, None));
        assert_eq!(resolve_range(..), (0, None));
        assert_eq!(resolve_range(0..5), (0, Some(5)));
        assert_eq!(resolve_range(..5), (0, Some(5)));
        assert_eq!(resolve_range(0..=5), (0, Some(6)));
        assert_eq!(resolve_range(..=5), (0, Some(6)));
        assert_eq!(resolve_range(10..5), (10, Some(0)));
        assert_eq!(resolve_range(10..=5), (10, Some(0)));
        assert_eq!(resolve_range(10..10), (10, Some(0)));
        assert_eq!(resolve_range(10..=10), (10, Some(1)));
        assert_eq!(resolve_range(10..11), (10, Some(1)));
        assert_eq!(resolve_range(10..=11), (10, Some(2)));
        assert_eq!(resolve_range(10..9), (10, Some(0)));
        assert_eq!(resolve_range(10..=9), (10, Some(0)));
        assert_eq!(resolve_range(10..), (10, None));
        assert_eq!(resolve_range(10..15), (10, Some(5)));
        assert_eq!(resolve_range(10..=15), (10, Some(6)));
    }

    #[rstest::rstest]
    fn test_iterator_simple(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig::default());

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(1)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), None);
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_empty_chain(empty_chain: Arc<MadaraBackend>) {
        let mut ite = empty_chain.block_info_iterator(BlockStreamConfig::default());
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_start_from_block(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig { start: 3, ..Default::default() });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), None);
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_start_from_not_yet_created(empty_chain: Arc<MadaraBackend>) {
        let mut ite = empty_chain.block_info_iterator(BlockStreamConfig { start: 3, ..Default::default() });
        assert_eq!(ite.next().transpose().unwrap(), None);

        store_block(&empty_chain, 0);
        store_block(&empty_chain, 1);
        let mut ite = empty_chain.block_info_iterator(BlockStreamConfig { start: 3, ..Default::default() });
        assert_eq!(ite.next().transpose().unwrap(), None);
        store_block(&empty_chain, 2);
        store_block(&empty_chain, 3);
        let mut ite = empty_chain.block_info_iterator(BlockStreamConfig { start: 3, ..Default::default() });
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), None);
        store_block(&empty_chain, 4);
        let mut ite = empty_chain.block_info_iterator(BlockStreamConfig { start: 3, ..Default::default() });
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_backward(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig {
            direction: Direction::Backward,
            start: 3,
            ..Default::default()
        });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(1)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_backward_empty(empty_chain: Arc<MadaraBackend>) {
        let mut ite = empty_chain.block_info_iterator(BlockStreamConfig {
            direction: Direction::Backward,
            start: 0,
            ..Default::default()
        });

        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_backward_start_from_not_yet_created(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig {
            direction: Direction::Backward,
            start: 10,
            ..Default::default()
        });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(1)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_step(test_chain: Arc<MadaraBackend>) {
        let mut ite =
            test_chain.block_info_iterator(BlockStreamConfig { step: 2.try_into().unwrap(), ..Default::default() });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), None);

        store_block(&test_chain, 5);
        let mut ite =
            test_chain.block_info_iterator(BlockStreamConfig { step: 2.try_into().unwrap(), ..Default::default() });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), None);

        store_block(&test_chain, 6);
        let mut ite =
            test_chain.block_info_iterator(BlockStreamConfig { step: 2.try_into().unwrap(), ..Default::default() });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(6)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_step_backward(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig {
            direction: Direction::Backward,
            step: 2.try_into().unwrap(),
            start: 4,
            ..Default::default()
        });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_limit(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig { limit: Some(3), ..Default::default() });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(0)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(1)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }

    #[rstest::rstest]
    fn test_iterator_limit_backward(test_chain: Arc<MadaraBackend>) {
        let mut ite = test_chain.block_info_iterator(BlockStreamConfig {
            direction: Direction::Backward,
            limit: Some(3),
            start: 5,
            ..Default::default()
        });

        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(4)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(3)));
        assert_eq!(ite.next().transpose().unwrap(), Some(block_info(2)));
        assert_eq!(ite.next().transpose().unwrap(), None);
    }
}
