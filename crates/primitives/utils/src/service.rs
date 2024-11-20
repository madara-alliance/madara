//! Service trait and combinators.

use anyhow::Context;
use std::panic;
use tokio::task::JoinSet;

/// The app is divided into services, with each service having a different responsability within the app.
/// Depending on the startup configuration, some services are enabled and some are disabled.
///
/// This trait enables launching nested services and groups.
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync {
    /// Default impl does not start any task.
    async fn start(
        &mut self,
        _join_set: &mut JoinSet<anyhow::Result<()>>,
        _cancellation_token: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_and_drive_to_end(mut self) -> anyhow::Result<()>
    where
        Self: Sized,
    {
        let mut join_set = JoinSet::new();
        let cancellation_token = tokio_util::sync::CancellationToken::new();
        self.start(&mut join_set, cancellation_token).await.context("Starting service")?;
        drive_joinset(join_set).await
    }
}

pub struct ServiceGroup {
    services: Vec<Box<dyn Service>>,
    join_set: Option<JoinSet<anyhow::Result<()>>>,
}

impl Default for ServiceGroup {
    fn default() -> Self {
        Self { services: vec![], join_set: Some(Default::default()) }
    }
}

impl ServiceGroup {
    pub fn new(services: Vec<Box<dyn Service>>) -> Self {
        Self { services, join_set: Some(Default::default()) }
    }

    /// Add a new service to the service group.
    pub fn push(&mut self, value: impl Service) {
        if self.join_set.is_none() {
            panic!("Cannot add services to a group that has been started.")
        }
        self.services.push(Box::new(value));
    }

    pub fn with(mut self, value: impl Service) -> Self {
        self.push(value);
        self
    }
}

#[async_trait::async_trait]
impl Service for ServiceGroup {
    async fn start(
        &mut self,
        join_set: &mut JoinSet<anyhow::Result<()>>,
        cancellation_token: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<()> {
        // drive the join set as a nested task
        let mut own_join_set = self.join_set.take().expect("Service has already been started.");
        for svc in self.services.iter_mut() {
            svc.start(&mut own_join_set, cancellation_token.clone()).await.context("Starting service")?;
        }

        join_set.spawn(drive_joinset(own_join_set));
        Ok(())
    }
}

async fn drive_joinset(mut join_set: JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(result) => result?,
            Err(panic_error) if panic_error.is_panic() => {
                // bubble up panics too
                panic::resume_unwind(panic_error.into_panic());
            }
            Err(_task_cancelled_error) => {}
        }
    }

    Ok(())
}
