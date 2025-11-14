use crate::worker::event_handler::factory::mock_factory;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

/// Mutex to protect access to the mock factory context
/// This ensures that only one test can access the mock context at a time,
/// preventing PoisonError when tests run in parallel.
/// Note: This is separate from TEST_MUTEX because tests hold TEST_MUTEX for the
/// entire test duration, and we need to lock the mock context separately within the test.
static MOCK_CONTEXT_MUTEX: Mutex<()> = Mutex::new(());

/// Test-level mutex to serialize tests that use mocks
/// This ensures only one test using mocks runs at a time, preventing interference
/// between tests when they set expectations for the same JobType.
/// This is held for the entire test duration via acquire_test_lock().
static TEST_MUTEX: Mutex<()> = Mutex::new(());

/// Wrapper that holds both the mutex guard and the mock context
/// This ensures the guard is held for the lifetime of the context usage
pub struct MockContextGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
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

/// Get the mock job handler context in a thread-safe way
/// This function ensures that only one test can set expectations at a time
/// by using a Mutex to serialize access to the global mock context.
/// Note: This uses MOCK_CONTEXT_MUTEX (not TEST_MUTEX) because tests already
/// hold TEST_MUTEX for the entire test duration, and we need a separate lock
/// for the mock context access within the test.
pub fn get_job_handler_context_safe() -> MockContextGuard {
    // Lock the mutex to ensure exclusive access
    // The guard will be held until MockContextGuard is dropped
    let guard = MOCK_CONTEXT_MUTEX.lock().unwrap_or_else(|e| {
        // If the mutex is poisoned (a test panicked while holding the lock),
        // recover by getting the inner value
        tracing::warn!("MOCK_CONTEXT_MUTEX was poisoned (a test panicked while holding the lock), recovering");
        e.into_inner()
    });

    // Get the context - the guard will be held until MockContextGuard is dropped
    // This ensures only one test can set expectations at a time
    let context = mock_factory::get_job_handler_context();

    MockContextGuard { _guard: guard, context }
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
