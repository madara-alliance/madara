use super::super::cloud::CloudProvider;
use crate::OrchestratorResult;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Resource trait
///
/// The Resource trait is used to define the interface for resources that can be used by the Orchestrator.
/// It provides a common interface for interacting with resources, such as databases, queues, and storage.
/// The trait is used to abstract away the specific implementation of the resource, allowing the Orchestrator
/// to work with a variety of resources without needing to know the specific details of each implementation.
/// The trait is also used to define the interface for mocking the resource, allowing the Orchestrator to
/// be tested in isolation without needing to set up a real resource.
#[async_trait]
pub trait Resource: Send + Sync {
    type SetupResult: Send + Sync;
    type CheckResult: Send + Sync;
    type TeardownResult: Send + Sync;
    type Error: Send + Sync;
    type SetupArgs: Send + Sync;
    type CheckArgs: Send + Sync;

    /// create_setup - create a new setup Reference
    async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self>
    where
        Self: Sized;

    /// setup - Setup the resource
    /// This function will check if the resource exists, if not create it.
    /// This function will also create any dependent resources that are needed.
    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult>;
    /// check - Check if the resource exists, check only for individual resources
    async fn check_if_exists(&self, args: Self::CheckArgs) -> OrchestratorResult<bool>;
    /// ready - Check if all the resource is created and ready to use
    async fn is_ready_to_use(&self, args: &Self::SetupArgs) -> OrchestratorResult<bool>;
    /// check - Check if the resource is ready to use
    /// This function will check if the resource is ready to use.
    async fn poll(&self, args: Self::SetupArgs, poll_interval: u64, timeout: u64) -> bool {
        let timeout_duration = Duration::from_secs(timeout);
        let start_time = Instant::now();

        while start_time.elapsed() < timeout_duration {
            match self.is_ready_to_use(&args).await {
                Ok(true) => return true,
                Ok(false) => {
                    tokio::time::sleep(Duration::from_secs(poll_interval)).await;
                    continue;
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(poll_interval)).await;
                    continue;
                }
            }
        }
        false
    }
}
