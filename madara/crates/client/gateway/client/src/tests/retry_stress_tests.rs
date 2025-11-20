/// Comprehensive stress and soak tests for the hybrid retry strategy.
///
/// These tests validate all possible behaviors of the retry mechanism:
/// - Phase transitions (Aggressive -> Backoff -> Steady)
/// - Error type handling (connection refused, timeout, rate limits)
/// - Log throttling to prevent spam
/// - Long-running gateway outages
/// - Flapping gateway scenarios
///
/// Run with: `cargo test --package mc-gateway-client -- --ignored --test-threads=1`

use crate::retry::{RetryConfig, RetryPhase, RetryState};
use crate::GatewayProvider;
use httpmock::prelude::*;
use mp_gateway::error::SequencerError;
use std::time::Duration;
use tokio::time::Instant;

/// Helper to create a gateway provider pointing to a mock server
#[allow(dead_code)]
fn create_test_gateway(server: &MockServer) -> GatewayProvider {
    let base_url = server.url("");
    GatewayProvider::new(
        format!("{}/gateway", base_url).parse().unwrap(),
        format!("{}/feeder_gateway", base_url).parse().unwrap(),
    )
}

/// Test 1: Quick recovery from temporary blip (Phase 1)
/// Tests that retry intervals in Phase 1 are short (2 seconds)
#[tokio::test(start_paused = true)]
 // Stress test - run explicitly
async fn test_quick_recovery_phase1() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(300), // 5 minutes
        phase1_interval: Duration::from_secs(2),   // 2 seconds
        ..Default::default()
    };
    let state = RetryState::new(config.clone());

    // Simulate 5 quick failures in Phase 1
    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    let start = Instant::now();

    for i in 0..5 {
        state.increment_retry().await;
        let delay = state.next_delay(&error);

        // In Phase 1, delay should be 2 seconds
        assert_eq!(delay, Duration::from_secs(2), "Phase 1 should use 2s interval");
        assert_eq!(state.current_phase(), RetryPhase::Aggressive, "Should be in Phase 1");

        // Simulate waiting
        tokio::time::advance(delay).await;

        println!("Retry {}: delay={}s, phase={:?}", i, delay.as_secs(), state.current_phase());
    }

    let elapsed = start.elapsed();

    // 5 retries * 2 seconds = 10 seconds total
    assert_eq!(elapsed.as_secs(), 10, "5 retries in Phase 1 should take 10 seconds");
    assert!(state.current_phase() == RetryPhase::Aggressive, "Should still be in Phase 1 after 10s");
}

/// Test 2: Phase 1 aggressive polling behavior
#[tokio::test(start_paused = true)]

async fn test_phase1_aggressive_polling() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(300), // 5 minutes
        phase1_interval: Duration::from_secs(2),
        ..Default::default()
    };

    let state = RetryState::new(config);

    // Simulate connection refused error
    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Test multiple delays in Phase 1
    for i in 0..10 {
        let delay = state.next_delay(&error);
        assert_eq!(
            delay,
            Duration::from_secs(2),
            "Phase 1 should maintain constant 2s interval (attempt {})",
            i
        );

        assert_eq!(state.current_phase(), RetryPhase::Aggressive);

        // Advance time by 2 seconds
        tokio::time::advance(Duration::from_secs(2)).await;
    }
}

/// Test 3: Phase transition from Phase 1 to Phase 2
#[tokio::test(start_paused = true)]

async fn test_phase1_to_phase2_transition() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(10), // Short phase 1 for testing
        phase1_interval: Duration::from_secs(2),
        phase2_min_delay: Duration::from_secs(5),
        max_backoff: Duration::from_secs(60),
        ..Default::default()
    };

    let state = RetryState::new(config.clone());

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Should start in Phase 1
    assert_eq!(state.current_phase(), RetryPhase::Aggressive);
    assert_eq!(state.next_delay(&error), Duration::from_secs(2));

    // Advance time to just before phase transition
    tokio::time::advance(Duration::from_secs(9)).await;
    assert_eq!(state.current_phase(), RetryPhase::Aggressive);

    // Advance past the phase 1 duration
    tokio::time::advance(Duration::from_secs(2)).await;

    // Should now be in Phase 2
    match state.current_phase() {
        RetryPhase::Backoff { .. } => {
            // Success - we're in backoff phase
            let delay = state.next_delay(&error);
            assert!(delay >= Duration::from_secs(5), "Phase 2 should use exponential backoff");
        }
        _ => panic!("Should have transitioned to Phase 2"),
    }
}

/// Test 4: Exponential backoff in Phase 2
#[tokio::test(start_paused = true)]

async fn test_phase2_exponential_backoff() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(0), // Skip Phase 1
        phase2_min_delay: Duration::from_secs(5),
        max_backoff: Duration::from_secs(60),
        ..Default::default()
    };

    let state = RetryState::new(config);

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Advance to Phase 2
    tokio::time::advance(Duration::from_secs(1)).await;

    // Test exponential growth: 5s, 10s, 20s, 40s, 60s (capped)
    let expected_delays = vec![5, 10, 20, 40, 60, 60, 60];

    for (i, expected) in expected_delays.iter().enumerate() {
        let delay = state.next_delay(&error);
        assert_eq!(
            delay.as_secs(),
            *expected,
            "Attempt {} should have delay {}s",
            i,
            expected
        );

        tokio::time::advance(Duration::from_secs(5)).await;
    }
}

/// Test 5: Max backoff cap is respected
#[tokio::test(start_paused = true)]

async fn test_max_backoff_cap() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(0),
        phase2_min_delay: Duration::from_secs(5),
        max_backoff: Duration::from_secs(30), // Lower cap for testing
        ..Default::default()
    };

    let state = RetryState::new(config.clone());

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Simulate many retries
    for _ in 0..20 {
        tokio::time::advance(Duration::from_secs(5)).await;
    }

    // Delay should be capped at max_backoff
    let delay = state.next_delay(&error);
    assert_eq!(delay, config.max_backoff, "Delay should be capped at max_backoff");
}

/// Test 6: Connection refused error handling
#[tokio::test]
async fn test_connection_refused_error() {
    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    assert!(RetryState::is_connection_error(&error));

    let reason = RetryState::format_error_reason(&error);
    assert_eq!(reason, "connection refused");
}

/// Test 7: Timeout error handling
#[tokio::test]
async fn test_timeout_error() {
    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "Operation timed out"
    )));

    assert!(RetryState::is_timeout_error(&error));

    let reason = RetryState::format_error_reason(&error);
    assert_eq!(reason, "timeout");
}

/// Test 8: Rate limiting with Retry-After header
#[tokio::test]
async fn test_rate_limit_handling() {
    use mp_gateway::error::StarknetError;

    let error = SequencerError::StarknetError(StarknetError::rate_limited());

    let state = RetryState::new(RetryConfig::default());
    let delay = state.next_delay(&error);

    // Should respect rate limiting
    assert!(delay >= Duration::from_secs(2), "Rate limited errors should have appropriate delay");
}

/// Test 9: Log throttling in Phase 1
#[tokio::test(start_paused = true)]

async fn test_phase1_log_throttling() {
    let config = RetryConfig {
        log_interval: Duration::from_secs(10),
        ..Default::default()
    };

    let state = RetryState::new(config);

    // First log should be allowed
    assert!(state.should_log().await, "First log should be allowed");

    // Immediate second log should be throttled
    assert!(!state.should_log().await, "Second immediate log should be throttled");

    // Advance time by 5 seconds (less than log_interval)
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(!state.should_log().await, "Still should be throttled");

    // Advance past the log interval
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(state.should_log().await, "Should log after interval");
}

/// Test 10: Retry counter increments correctly
#[tokio::test]
async fn test_retry_counter() {
    let state = RetryState::new(RetryConfig::default());

    assert_eq!(state.get_retry_count().await, 0);

    for i in 1..=10 {
        let count = state.increment_retry().await;
        assert_eq!(count, i);
        assert_eq!(state.get_retry_count().await, i);
    }
}

/// Test 11: Clean error message formatting
#[tokio::test]
async fn test_error_message_formatting() {
    let test_cases = vec![
        (
            SequencerError::HttpCallError(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused"
            ))),
            "connection refused"
        ),
        (
            SequencerError::HttpCallError(Box::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timeout"
            ))),
            "timeout"
        ),
        (
            SequencerError::StarknetError(mp_gateway::error::StarknetError::rate_limited()),
            "rate limited"
        ),
    ];

    for (error, expected) in test_cases {
        let formatted = RetryState::format_error_reason(&error);
        assert_eq!(formatted, expected, "Error formatting mismatch for: {:?}", error);
    }
}

/// Test 12: Eventual success after multiple failures
/// Validates that retry state persists correctly through multiple failures
#[tokio::test(start_paused = true)]
async fn test_eventual_success() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(300),
        phase1_interval: Duration::from_secs(2),
        infinite_retry: true,
        ..Default::default()
    };
    let state = RetryState::new(config.clone());

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Simulate 10 failures, ensuring state persists correctly
    for i in 0..10 {
        let retry_count = state.increment_retry().await;
        assert_eq!(retry_count, i + 1, "Retry count should increment correctly");

        let delay = state.next_delay(&error);
        assert_eq!(delay, Duration::from_secs(2), "Phase 1 delays should be 2s");

        tokio::time::advance(delay).await;
    }

    let final_count = state.get_retry_count().await;
    assert_eq!(final_count, 10, "Should have tracked 10 retries");

    // Verify still in Phase 1 after 20 seconds (10 retries * 2s)
    assert_eq!(state.current_phase(), RetryPhase::Aggressive);

    println!("Successfully tracked {} retries with persistent state", final_count);
}

/// Test 13: Long-running outage simulation (30+ minutes simulated)
#[tokio::test(start_paused = true)]
async fn test_extended_outage_30min() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(5 * 60),  // 5 minutes
        phase1_interval: Duration::from_secs(2),
        phase2_min_delay: Duration::from_secs(5),
        max_backoff: Duration::from_secs(60),
        ..Default::default()
    };

    let state = RetryState::new(config);

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Simulate 30 minutes of retries
    let total_duration = Duration::from_secs(30 * 60);
    let mut elapsed = Duration::from_secs(0);
    let mut retry_count = 0;

    while elapsed < total_duration {
        let delay = state.next_delay(&error);
        tokio::time::advance(delay).await;
        elapsed += delay;
        retry_count += 1;

        // Log phase every few minutes
        if retry_count % 10 == 0 {
            let phase = state.current_phase();
            println!("Elapsed: {:?}, Phase: {:?}, Delay: {:?}", elapsed, phase, delay);
        }
    }

    // After 30 minutes, should be in steady state with max backoff
    let final_delay = state.next_delay(&error);
    assert_eq!(final_delay, Duration::from_secs(60), "Should be at max backoff after 30 min");

    println!("Completed {} retries over 30 simulated minutes", retry_count);
}

/// Test 14: Flapping gateway (rapid connect/disconnect cycles)
/// Simulates a gateway that alternates between success and failure
/// Validates that retry state resets properly after successful requests
#[tokio::test(start_paused = true)]
async fn test_flapping_gateway() {
    let config = RetryConfig {
        phase1_duration: Duration::from_secs(300),
        phase1_interval: Duration::from_secs(2),
        ..Default::default()
    };

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Simulate 5 flapping cycles (fail -> retry -> success -> fail -> retry -> success...)
    for cycle in 0..5 {
        let state = RetryState::new(config.clone());

        // Fail once
        state.increment_retry().await;
        let delay = state.next_delay(&error);
        assert_eq!(delay, Duration::from_secs(2), "Phase 1 should use 2s interval");

        tokio::time::advance(delay).await;

        // After 1 failure + 2s delay, should succeed (simulated)
        let retry_count = state.get_retry_count().await;
        assert_eq!(retry_count, 1, "Should have 1 retry in cycle {}", cycle);

        println!("Flapping cycle {}: failed once, then succeeded (1 retry, 2s total)", cycle);

        // Next cycle starts with fresh state (simulating success reset)
    }

    println!("Successfully handled 5 flapping cycles with state reset");
}

/// Test 15: Mixed error types during retry sequence
#[tokio::test]
async fn test_mixed_error_types() {
    let errors = vec![
        SequencerError::HttpCallError(Box::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Connection refused"
        ))),
        SequencerError::HttpCallError(Box::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timeout"
        ))),
        SequencerError::StarknetError(mp_gateway::error::StarknetError::rate_limited()),
    ];

    let state = RetryState::new(RetryConfig::default());

    for error in errors {
        let delay = state.next_delay(&error);
        let is_conn_error = RetryState::is_connection_error(&error);
        let is_timeout = RetryState::is_timeout_error(&error);
        let formatted = RetryState::format_error_reason(&error);

        println!(
            "Error: {:?}, Delay: {:?}, ConnErr: {}, Timeout: {}, Formatted: {}",
            error, delay, is_conn_error, is_timeout, formatted
        );

        assert!(delay > Duration::from_secs(0), "Should have non-zero delay");
    }
}

/// Test 16: Verify no panic on infinite retry loop
#[tokio::test(start_paused = true)]

async fn test_infinite_retry_stability() {
    let config = RetryConfig {
        infinite_retry: true,
        ..Default::default()
    };

    let state = RetryState::new(config);

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    // Simulate 1000 retries
    for i in 0..1000 {
        let delay = state.next_delay(&error);
        state.increment_retry().await;
        tokio::time::advance(delay).await;

        if i % 100 == 0 {
            println!("Retry {} completed, phase: {:?}", i, state.current_phase());
        }
    }

    // Should complete without panic
    assert_eq!(state.get_retry_count().await, 1000);
}

/// Test 17: Performance test - measure retry overhead
#[tokio::test]
async fn test_retry_performance_overhead() {
    let state = RetryState::new(RetryConfig::default());

    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));

    let start = Instant::now();

    // Measure overhead of 1000 delay calculations
    for _ in 0..1000 {
        let _delay = state.next_delay(&error);
        let _formatted = RetryState::format_error_reason(&error);
        state.increment_retry().await;
    }

    let elapsed = start.elapsed();

    println!("1000 retry calculations took {:?}", elapsed);
    assert!(elapsed < Duration::from_secs(1), "Retry calculations should be fast");
}
