//! Service trait and combinators.

use anyhow::Context;
use futures::Future;
use std::{borrow::Cow, panic};
use tokio::task::{AbortHandle, JoinSet};

/// Wrapper around a [`JoinSet`], but it associates tasks with their name so that error reporting has context.
pub struct TaskGroup {
    set: JoinSet<anyhow::Result<()>>,
    name: Cow<'static, str>,
}

impl TaskGroup {
    pub fn new(name: impl Into<Cow<'static, str>>) -> Self {
        Self { set: JoinSet::new(), name: name.into() }
    }

    #[track_caller]
    pub fn spawn<F>(&mut self, task: F) -> AbortHandle
    where
        F: Future<Output = anyhow::Result<()>>,
        F: Send + 'static,
    {
        let name = self.name.clone();
        self.set.spawn(async move { task.await.with_context(|| format!("Running task for service: {}", name)) })
    }
}

/// The app is divided into services, with each service having a different responsability within the app.
/// Depending on the startup configuration, some services are enabled and some are disabled.
///
/// This trait enables launching nested services and groups.
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync {
    /// Default impl does not start any task.
    async fn start(&mut self, _join_set: &mut TaskGroup) -> anyhow::Result<()> {
        Ok(())
    }

    fn name(&self) -> &str {
        "<unnamed>"
    }

    async fn start_and_drive_to_end(mut self) -> anyhow::Result<()>
    where
        Self: Sized,
    {
        let mut join_set = TaskGroup::new(self.name().to_owned());
        self.start(&mut join_set).await.with_context(|| format!("Starting service: {}", self.name()))?;
        drive_joinset(join_set.set).await
    }
}

pub struct ServiceGroup {
    services: Vec<Box<dyn Service>>,
    join_set: Option<TaskGroup>,
    name: Cow<'static, str>,
}

impl Default for ServiceGroup {
    fn default() -> Self {
        let name: Cow<_> = "<unnamed service group>".into();
        Self { services: vec![], join_set: Some(TaskGroup::new(name.clone())), name }
    }
}

impl ServiceGroup {
    pub fn new(services: Vec<Box<dyn Service>>) -> Self {
        ServiceGroup { services, ..Default::default() }
    }

    pub fn named(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.name = name.into();
        self
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
    async fn start(&mut self, join_set: &mut TaskGroup) -> anyhow::Result<()> {
        // drive the join set as a nested task
        let mut own_join_set = self.join_set.take().expect("Service has already been started.");
        own_join_set.name.clone_from(&self.name);

        for svc in self.services.iter_mut() {
            let mut group = TaskGroup::new(svc.name().to_owned());
            group.set = own_join_set.set;
            svc.start(&mut group).await.with_context(|| format!("Starting service: {}", svc.name()))?;
            own_join_set.set = group.set;
        }

        join_set.spawn(drive_joinset(own_join_set.set));
        Ok(())
    }

    fn name(&self) -> &str {
        self.name.as_ref()
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
