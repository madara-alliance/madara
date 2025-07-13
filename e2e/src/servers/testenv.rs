use std::sync::{Arc, Mutex, OnceLock};
use std::collections::HashMap;
use tokio::sync::{Mutex as TokioMutex, RwLock};
use std::time::Duration;
use uuid::Uuid;

// =============================================================================
// GLOBAL TEST ENVIRONMENT MANAGER
// =============================================================================

/// Global registry for test environments - ensures environments persist
/// across test boundaries and can be shared between tests
static TEST_ENVIRONMENTS: OnceLock<Arc<RwLock<HashMap<String, Arc<TestEnvironment>>>>> = OnceLock::new();

/// Get or initialize the global test environment registry
fn get_test_registry() -> &'static Arc<RwLock<HashMap<String, Arc<TestEnvironment>>>> {
    TEST_ENVIRONMENTS.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

// =============================================================================
// TEST ENVIRONMENT - Manages lifecycle of all dependencies
// =============================================================================

#[derive(Debug)]
pub struct TestEnvironment {
    pub id: String,
    pub setup: Arc<TokioMutex<Setup>>,
    pub config: SetupConfig,
    pub created_at: std::time::Instant,
    pub ref_count: Arc<Mutex<usize>>,
    pub shutdown_handle: Arc<TokioMutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl TestEnvironment {
    /// Create a new test environment with unique ID
    pub async fn create(config: SetupConfig) -> Result<Arc<Self>, SetupError> {
        let env_id = format!("test-env-{}", Uuid::new_v4());
        
        println!("🚀 Creating test environment: {}", env_id);
        
        let mut setup = Setup::new(config.clone())?;
        setup.setup().await?;
        
        let env = Arc::new(TestEnvironment {
            id: env_id.clone(),
            setup: Arc::new(TokioMutex::new(setup)),
            config,
            created_at: std::time::Instant::now(),
            ref_count: Arc::new(Mutex::new(1)),
            shutdown_handle: Arc::new(TokioMutex::new(None)),
        });
        
        // Register in global registry
        let registry = get_test_registry();
        let mut registry_lock = registry.write().await;
        registry_lock.insert(env_id, env.clone());
        
        // Setup automatic cleanup after test timeout
        let cleanup_env = env.clone();
        let cleanup_handle = tokio::spawn(async move {
            // Wait for reasonable test timeout (configurable)
            tokio::time::sleep(Duration::from_secs(1800)).await; // 30 minutes
            cleanup_env.cleanup().await;
        });
        
        *env.shutdown_handle.lock().await = Some(cleanup_handle);
        
        println!("✅ Test environment created: {}", env.id);
        Ok(env)
    }
    
    /// Get or create a shared test environment (for test reuse)
    pub async fn get_or_create_shared(
        name: &str, 
        config: SetupConfig
    ) -> Result<Arc<Self>, SetupError> {
        let registry = get_test_registry();
        
        // Try to get existing environment
        {
            let registry_lock = registry.read().await;
            if let Some(env) = registry_lock.get(name) {
                // Increment reference count
                {
                    let mut ref_count = env.ref_count.lock().unwrap();
                    *ref_count += 1;
                }
                println!("♻️  Reusing existing test environment: {}", name);
                return Ok(env.clone());
            }
        }
        
        // Create new environment with specific name
        let mut setup = Setup::new(config.clone())?;
        setup.setup().await?;
        
        let env = Arc::new(TestEnvironment {
            id: name.to_string(),
            setup: Arc::new(TokioMutex::new(setup)),
            config,
            created_at: std::time::Instant::now(),
            ref_count: Arc::new(Mutex::new(1)),
            shutdown_handle: Arc::new(TokioMutex::new(None)),
        });
        
        // Register in global registry
        {
            let mut registry_lock = registry.write().await;
            registry_lock.insert(name.to_string(), env.clone());
        }
        
        println!("✅ Created new shared test environment: {}", name);
        Ok(env)
    }
    
    /// Get endpoints for services
    pub async fn get_endpoints(&self) -> TestEndpoints {
        let setup = self.setup.lock().await;
        TestEndpoints {
            anvil: setup.anvil.as_ref().map(|s| s.server().endpoint().to_string()),
            madara: setup.madara.as_ref().map(|s| s.endpoint().to_string()),
            localstack: setup.localstack.as_ref().map(|s| s.server().endpoint().to_string()),
            mongo: setup.mongo.as_ref().map(|s| format!("mongodb://127.0.0.1:{}", s.server().port())),
            pathfinder: setup.pathfinder.as_ref().map(|s| s.endpoint().to_string()),
        }
    }
    
    /// Check if all services are healthy
    pub async fn health_check(&self) -> Result<HealthStatus, SetupError> {
        let setup = self.setup.lock().await;
        let mut status = HealthStatus::default();
        
        // Check each service
        if let Some(anvil) = &setup.anvil {
            status.anvil = anvil.server().is_running();
        }
        
        if let Some(madara) = &setup.madara {
            // You could add a health check method to MadaraService
            status.madara = true; // Placeholder
        }
        
        // Add checks for other services...
        
        Ok(status)
    }
    
    /// Cleanup the test environment
    pub async fn cleanup(&self) {
        println!("🧹 Cleaning up test environment: {}", self.id);
        
        // Cancel the cleanup handle
        if let Some(handle) = self.shutdown_handle.lock().await.take() {
            handle.abort();
        }
        
        // Cleanup services
        let mut setup = self.setup.lock().await;
        if let Some(mut anvil) = setup.anvil.take() {
            let _ = anvil.server_mut().stop();
        }
        
        // Remove from registry
        let registry = get_test_registry();
        let mut registry_lock = registry.write().await;
        registry_lock.remove(&self.id);
        
        println!("✅ Test environment cleaned up: {}", self.id);
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        // Decrement reference count
        let mut ref_count = self.ref_count.lock().unwrap();
        *ref_count -= 1;
        
        // If this was the last reference, schedule cleanup
        if *ref_count == 0 {
            let env_id = self.id.clone();
            println!("📉 Last reference dropped for environment: {}", env_id);
            
            // Spawn cleanup task (can't await in Drop)
            tokio::spawn(async move {
                let registry = get_test_registry();
                let mut registry_lock = registry.write().await;
                if let Some(env) = registry_lock.remove(&env_id) {
                    env.cleanup().await;
                }
            });
        }
    }
}

// =============================================================================
// SUPPORTING TYPES
// =============================================================================

#[derive(Debug, Clone)]
pub struct TestEndpoints {
    pub anvil: Option<String>,
    pub madara: Option<String>,
    pub localstack: Option<String>,
    pub mongo: Option<String>,
    pub pathfinder: Option<String>,
}

#[derive(Debug, Default)]
pub struct HealthStatus {
    pub anvil: bool,
    pub madara: bool,
    pub localstack: bool,
    pub mongo: bool,
    pub pathfinder: bool,
}

impl HealthStatus {
    pub fn all_healthy(&self) -> bool {
        self.anvil && self.madara && self.localstack && self.mongo && self.pathfinder
    }
}

// =============================================================================
// TEST FIXTURE WRAPPER
// =============================================================================

/// Wrapper that ensures the test environment stays alive for the test duration
pub struct TestHandle {
    pub env: Arc<TestEnvironment>,
    pub endpoints: TestEndpoints,
    _phantom: std::marker::PhantomData<()>, // Prevents direct construction
}

impl TestHandle {
    /// Create a new test handle (private - use fixtures)
    async fn new(env: Arc<TestEnvironment>) -> Self {
        let endpoints = env.get_endpoints().await;
        Self {
            env,
            endpoints,
            _phantom: std::marker::PhantomData,
        }
    }
    
    /// Get the setup instance (with proper locking)
    pub async fn setup(&self) -> tokio::sync::MutexGuard<'_, Setup> {
        self.env.setup.lock().await
    }
    
    /// Execute a test with the setup
    pub async fn with_setup<F, Fut, R>(&self, test_fn: F) -> R
    where
        F: FnOnce(&mut Setup) -> Fut,
        Fut: std::future::Future<Output = R>,
    {
        let mut setup = self.env.setup.lock().await;
        test_fn(&mut *setup).await
    }
    
    /// Check if all services are healthy
    pub async fn ensure_healthy(&self) -> Result<(), SetupError> {
        let status = self.env.health_check().await?;
        if !status.all_healthy() {
            return Err(SetupError::StartupFailed("Some services are unhealthy".to_string()));
        }
        Ok(())
    }
}

// =============================================================================
// RSTEST FIXTURES
// =============================================================================

/// Creates a fresh test environment for each test
#[rstest::fixture]
pub async fn fresh_test_env() -> TestHandle {
    let config = SetupConfig::default();
    let env = TestEnvironment::create(config)
        .await
        .expect("Failed to create test environment");
    
    TestHandle::new(env).await
}

/// Creates a shared test environment that can be reused across tests
#[rstest::fixture]
pub async fn shared_test_env() -> TestHandle {
    let config = SetupConfig::default();
    let env = TestEnvironment::get_or_create_shared("shared-default", config)
        .await
        .expect("Failed to create shared test environment");
    
    TestHandle::new(env).await
}

/// Creates a test environment with specific configuration
#[rstest::fixture]
pub async fn custom_test_env(
    #[default(8545)] anvil_port: u16,
    #[default(9944)] madara_port: u16,
) -> TestHandle {
    let config = SetupConfig {
        anvil_port,
        madara_port,
        ..Default::default()
    };
    
    let env = TestEnvironment::create(config)
        .await
        .expect("Failed to create custom test environment");
    
    TestHandle::new(env).await
}

// =============================================================================
// EXAMPLE TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[tokio::test]
    async fn test_fresh_environment(fresh_test_env: TestHandle) {
        // Each test gets a completely fresh environment
        fresh_test_env.ensure_healthy().await.unwrap();
        
        println!("🧪 Testing with fresh environment: {}", fresh_test_env.env.id);
        println!("📡 Anvil endpoint: {:?}", fresh_test_env.endpoints.anvil);
        
        // Your actual test logic here
        fresh_test_env.with_setup(|setup| async {
            // Test with setup
            assert!(setup.anvil.is_some());
        }).await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_shared_environment(shared_test_env: TestHandle) {
        // Multiple tests can share the same environment for efficiency
        shared_test_env.ensure_healthy().await.unwrap();
        
        println!("🧪 Testing with shared environment: {}", shared_test_env.env.id);
        
        // Test that shares environment with other tests using same fixture
        shared_test_env.with_setup(|setup| async {
            assert!(setup.anvil.is_some());
        }).await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_custom_ports(#[with(8546, 9945)] custom_test_env: TestHandle) {
        // Test with custom configuration
        custom_test_env.ensure_healthy().await.unwrap();
        
        println!("🧪 Testing with custom ports: {}", custom_test_env.env.id);
        assert!(custom_test_env.endpoints.anvil.as_ref().unwrap().contains("8546"));
    }

    #[rstest]
    #[tokio::test]
    async fn test_sequential_operations(shared_test_env: TestHandle) {
        // Test that performs multiple operations in sequence
        shared_test_env.ensure_healthy().await.unwrap();
        
        // Setup phase
        shared_test_env.with_setup(|setup| async {
            // Perform setup operations
            println!("Setting up test state...");
        }).await;
        
        // Test phase
        shared_test_env.with_setup(|setup| async {
            // Perform actual test
            println!("Running test operations...");
        }).await;
        
        // Verification phase
        shared_test_env.with_setup(|setup| async {
            // Verify results
            println!("Verifying test results...");
        }).await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_parallel_operations() {
        // Test that creates multiple environments for parallel testing
        let config1 = SetupConfig { anvil_port: 8547, madara_port: 9946, ..Default::default() };
        let config2 = SetupConfig { anvil_port: 8548, madara_port: 9947, ..Default::default() };
        
        let (env1, env2) = tokio::try_join!(
            TestEnvironment::create(config1),
            TestEnvironment::create(config2)
        ).unwrap();
        
        let (handle1, handle2) = tokio::join!(
            TestHandle::new(env1),
            TestHandle::new(env2)
        );
        
        // Run tests in parallel
        tokio::try_join!(
            handle1.ensure_healthy(),
            handle2.ensure_healthy()
        ).unwrap();
        
        println!("🧪 Both environments are healthy and running in parallel");
    }
}

// =============================================================================
// UTILITY FUNCTIONS
// =============================================================================

/// Cleanup all test environments (useful for test teardown)
pub async fn cleanup_all_test_environments() {
    let registry = get_test_registry();
    let mut registry_lock = registry.write().await;
    
    let environments: Vec<_> = registry_lock.values().cloned().collect();
    registry_lock.clear();
    
    for env in environments {
        env.cleanup().await;
    }
    
    println!("🧹 All test environments cleaned up");
}

/// Get statistics about active test environments
pub async fn get_test_environment_stats() -> TestEnvironmentStats {
    let registry = get_test_registry();
    let registry_lock = registry.read().await;
    
    TestEnvironmentStats {
        total_environments: registry_lock.len(),
        environment_ids: registry_lock.keys().cloned().collect(),
        oldest_environment: registry_lock.values()
            .min_by_key(|env| env.created_at)
            .map(|env| env.id.clone()),
    }
}

#[derive(Debug)]
pub struct TestEnvironmentStats {
    pub total_environments: usize,
    pub environment_ids: Vec<String>,
    pub oldest_environment: Option<String>,
}