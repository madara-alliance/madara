use crate::worker::event_handler::factory::mock_factory;
use std::ops::{Deref, DerefMut};

/// Test-level mutex to serialize tests that use mocks
/// This ensures only one test using mocks runs at a time, preventing interference
/// between tests when they set expectations for the same JobType.
/// This is held for the entire test duration via acquire_test_lock().
///
/// Since all tests that use get_job_handler_context_safe() also use acquire_test_lock(),
/// we only need this single mutex to serialize test execution and protect the mock context.
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Wrapper that holds the mock context
/// The context is protected by TEST_MUTEX which must be acquired via acquire_test_lock()
/// before calling get_job_handler_context_safe()
pub struct MockContextGuard {
    context: mock_factory::__get_job_handler::Context,
}

impl Deref for MockContextGuard {
    type Target = mock_factory::__get_job_handler::Context;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

impl DerefMut for MockContextGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}

/// Get the mock job handler context
///
/// # Safety
/// This function assumes that TEST_MUTEX is already held via acquire_test_lock().
/// All tests that use this function must call acquire_test_lock() at the start of the test.
/// Since TEST_MUTEX serializes entire tests, no additional locking is needed here.
pub fn get_job_handler_context_safe() -> MockContextGuard {
    // Get the context - TEST_MUTEX (acquired via acquire_test_lock()) ensures
    // only one test can access this at a time
    let context = mock_factory::get_job_handler_context();

    MockContextGuard { context }
}

/// Acquire a test-level lock to serialize tests that use mocks
/// This ensures only one test using mocks runs at a time, preventing interference
/// Call this at the start of tests that use mocks, and the lock will be held
/// for the entire test duration
pub fn acquire_test_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_MUTEX.lock().unwrap_or_else(|e| {
        // If the mutex is poisoned (a test panicked while holding the lock),
        // recover by getting the inner value
        tracing::warn!("TEST_MUTEX was poisoned (a test panicked while holding the lock), recovering");
        e.into_inner()
    })
}
