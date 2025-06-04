use crate::{client::SettlementClientTrait, error::SettlementClientError};
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};

/// When starting the node for the first time, we want to replay recent messages from the core contract that we may have missed.
/// To do that, we have to find the first block_n which has a timestamp greater than `now - replay_max_duration`.
/// This function returns that block_n. This block_n will then be used as the first block from which messages are replayed.
pub async fn find_replay_block_n_start(
    settlement_client: &Arc<dyn SettlementClientTrait>,
    replay_max_duration: Duration,
    latest_block_n: u64,
) -> Result<u64, SettlementClientError> {
    let current_timestamp_secs =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("Current time is before UNIX_EPOCH").as_secs();
    let settlement_client_ = settlement_client.clone();
    let fun = move |block_n| {
        let settlement_client_ = settlement_client_.clone();
        async move { settlement_client_.get_block_n_timestamp(block_n).await }
    };
    get_messaging_block_n_start_impl(latest_block_n, fun, current_timestamp_secs, replay_max_duration).await
}

async fn get_messaging_block_n_start_impl<E, Fut: Future<Output = Result<u64, E>>>(
    latest_block_n: u64,
    mut get_block_timestamp: impl FnMut(u64) -> Fut,
    current_timestamp_secs: u64,
    replay_max_duration: Duration,
) -> Result<u64, E> {
    // We try to optimize the number of calls to the L1 node a little bit. This helps for setups that use a free rpc provider.
    // (mainly for devnets/local testing)
    // But please note that in prod you really should use a proper RPC provider for L1.

    let target_timestamp = current_timestamp_secs.saturating_sub(replay_max_duration.as_secs());

    // Find lower bound by exponential search.
    let (low, high) = {
        let mut low = latest_block_n;
        let mut step = 1;

        loop {
            let candidate = low.saturating_sub(step);
            let candidate_timestamp = get_block_timestamp(candidate).await?;

            low = candidate;
            step *= 2; // Double step size

            if candidate_timestamp <= target_timestamp || candidate == 0 {
                break;
            }
        }
        (low, latest_block_n)
    };

    // Binary search for exact answer.
    let mut left = low;
    let mut right = high;

    while left < right {
        let mid = left + (right - left) / 2;
        let mid_timestamp = get_block_timestamp(mid).await?;

        if mid_timestamp > target_timestamp {
            right = mid;
        } else {
            left = mid + 1;
        }
    }

    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    pub struct TestCase {
        pub block_timestamps: HashMap<u64, u64>,
        pub latest_block_n: u64,
    }

    impl TestCase {
        pub fn with_regular_blocks(latest_block: u64, block_time_secs: u64, latest_timestamp: u64) -> Self {
            let mut block_timestamps = HashMap::new();

            for i in 0..=latest_block {
                let blocks_from_latest = latest_block - i;
                let timestamp = latest_timestamp.saturating_sub(blocks_from_latest * block_time_secs);
                block_timestamps.insert(i, timestamp);
            }

            TestCase { block_timestamps, latest_block_n: latest_block }
        }

        pub fn with_custom_timestamps(timestamps: Vec<(u64, u64)>) -> Self {
            let mut block_timestamps = HashMap::new();
            let mut latest_block_n = 0;

            for (block_n, timestamp) in timestamps {
                block_timestamps.insert(block_n, timestamp);
                latest_block_n = latest_block_n.max(block_n);
            }

            TestCase { block_timestamps, latest_block_n }
        }

        pub fn run(&self, current_timestamp_secs: u64, replay_max_duration: Duration) -> u64 {
            futures::executor::block_on(get_messaging_block_n_start_impl(
                self.latest_block_n,
                |block_n| async move { Ok::<_, ()>(self.block_timestamps[&block_n]) },
                current_timestamp_secs,
                replay_max_duration,
            ))
            .expect("Test should succeed")
        }

        pub fn verify_result(&self, result: u64, target_timestamp: u64) {
            let result_timestamp = self.block_timestamps[&result];

            // Main invariant: result block timestamp > target
            if result != self.latest_block_n {
                // special case: algorithm returns latest when all timestamps are <
                assert!(
                    result_timestamp > target_timestamp,
                    "Result block {} (timestamp {}) should be > target timestamp {}",
                    result,
                    result_timestamp,
                    target_timestamp
                );
            }

            // If not the first block, previous block should be <= target
            if result > 0 && self.block_timestamps.contains_key(&(result - 1)) {
                let prev_timestamp = self.block_timestamps[&(result - 1)];
                assert!(
                    prev_timestamp <= target_timestamp,
                    "Previous block {} (timestamp {}) should be <= target timestamp {}",
                    result - 1,
                    prev_timestamp,
                    target_timestamp
                );
            }
        }
    }

    #[test]
    fn test_perfect_estimate() {
        // Scenario: Initial guess lands exactly on the target block
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 1000;
        let latest_timestamp = current_time - 60; // Latest block is 1 minute old
        let replay_duration = Duration::from_secs(120); // Want last 2 minutes

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Perfect estimate: found block {}", result);
    }

    #[test]
    fn test_guess_too_recent() {
        // Scenario: Need to search backwards because initial guess is too recent
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 1000;
        let latest_timestamp = current_time - 30; // Latest block is 30 seconds old
        let replay_duration = Duration::from_secs(600); // Want last 10 minutes

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Guess too recent: found block {}", result);
    }

    #[test]
    fn test_guess_too_old() {
        // Scenario: Initial guess is too old, need to search forward
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 500;
        let latest_timestamp = current_time - 10; // Very recent latest block
        let replay_duration = Duration::from_secs(60); // Short replay duration

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Guess too old: found block {}", result);
    }

    #[test]
    fn test_very_short_replay_duration() {
        // Scenario: Replay duration shorter than average block time
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 1000;
        let latest_timestamp = current_time - 20;
        let replay_duration = Duration::from_secs(5); // Shorter than block time

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Very short duration: found block {}", result);
    }

    #[test]
    fn test_very_long_replay_duration() {
        // Scenario: Replay duration goes back to genesis or before
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 100; // Small chain
        let latest_timestamp = current_time - 60;
        let replay_duration = Duration::from_secs(50000); // Much longer than chain history

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        // Should return block 0 or close to it
        assert!(result <= 5, "Should return early block for very long duration");

        println!("Very long duration: found block {}", result);
    }

    #[test]
    fn test_zero_replay_duration() {
        // Scenario: No replay needed
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 1000;
        let latest_timestamp = current_time - 60;
        let replay_duration = Duration::from_secs(0);

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Zero duration: found block {}", result);
    }

    #[test]
    fn test_irregular_block_times() {
        // Scenario: Block times vary significantly
        let current_time = 1000000;
        let mut timestamps = Vec::new();
        let mut time: u64 = current_time - 30;

        // Create blocks with irregular timing: [5s, 25s, 8s, 40s, 12s, ...]
        let block_times = [5, 25, 8, 40, 12, 15, 3, 30, 20, 10];
        for (i, &block_time) in block_times.iter().enumerate().rev() {
            timestamps.push((i as u64, time));
            time = time.saturating_sub(block_time);
        }

        let test_case = TestCase::with_custom_timestamps(timestamps);
        let replay_duration = Duration::from_secs(100);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Irregular blocks: found block {}", result);
    }

    #[test]
    fn test_network_lag_scenario() {
        // Scenario: Current time is behind latest block timestamp (network lag)
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 1000;
        let latest_timestamp = current_time + 120; // Latest block is 2 minutes in the "future"
        let replay_duration = Duration::from_secs(300);

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Network lag: found block {}", result);
    }

    #[test]
    fn test_single_block_chain() {
        // Scenario: Only genesis block exists
        let current_time = 1000000;
        let genesis_timestamp = current_time - 3600; // 1 hour ago

        let test_case = TestCase::with_custom_timestamps(vec![(0, genesis_timestamp)]);
        let replay_duration = Duration::from_secs(1800); // 30 minutes
        let result = test_case.run(current_time, replay_duration);

        // Should return the only block that exists
        assert_eq!(result, 0);

        println!("Single block: found block {}", result);
    }

    #[test]
    fn test_exactly_one_block_time() {
        // Scenario: Replay duration equals exactly one estimated block time
        let current_time = 1000000;
        let block_time = 15;
        let latest_block = 1000;
        let latest_timestamp = current_time - 45;
        let replay_duration = Duration::from_secs(15); // Exactly one block time

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Exactly one block time: found block {}", result);
    }

    #[test]
    fn test_latest_block_very_recent() {
        // Scenario: Latest block was just mined
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 1000;
        let latest_timestamp = current_time - 2; // Just 2 seconds ago
        let replay_duration = Duration::from_secs(120);

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        let target_timestamp = current_time - replay_duration.as_secs();
        test_case.verify_result(result, target_timestamp);

        println!("Latest block very recent: found block {}", result);
    }

    #[test]
    fn test_latest_block_very_old() {
        // Scenario: Chain is stale, latest block is old
        let current_time = 1000000;
        let block_time = 12;
        let latest_block = 500;
        let latest_timestamp = current_time - 7200; // 2 hours old
        let replay_duration = Duration::from_secs(3600); // Want last hour

        let test_case = TestCase::with_regular_blocks(latest_block, block_time, latest_timestamp);
        let result = test_case.run(current_time, replay_duration);

        // Since all blocks are older than target, should return the latest block
        assert_eq!(result, latest_block);

        println!("Latest block very old: found block {}", result);
    }
}
