//! Service trait and combinators.

use anyhow::Context;
use std::{panic, sync::Arc};
use tokio::task::JoinSet;

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum MadaraCapability {
    Database = 1,
    L1Sync = 2,
    L2Sync = 4,
    BlockProduction = 8,
    Rpc = 16,
    Gateway = 32,
    Telemetry = 64,
}

#[repr(transparent)]
#[derive(Default)]
pub struct MadaraCapabilitiesMask(std::sync::atomic::AtomicU8);

impl MadaraCapabilitiesMask {
    pub fn is_active(&self, cap: u8) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) | cap as u8 > 0
    }

    pub fn activate(&self, cap: MadaraCapability) -> bool {
        let prev = self.0.fetch_or(cap as u8, std::sync::atomic::Ordering::Acquire);
        prev | cap as u8 > 0
    }
}

#[repr(u8)]
#[derive(Default)]
pub enum MadaraState {
    #[default]
    Starting,
    Warp,
    Running,
    Shutdown,
}

impl From<u8> for MadaraState {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Starting,
            1 => Self::Warp,
            2 => Self::Running,
            _ => Self::Shutdown,
        }
    }
}

/// Atomic state and cancellation context associated to a Service.
///
/// # Scope
///
/// You can create a hierarchy of services by calling `ServiceContext::branch_local`.
/// Services are said to be in the same _local scope_ if they inherit the same
/// `token_local` cancellation token. You can think of services being local
/// if they can cancel each other without affecting the rest of the app (this
/// is not exact but it serves as a good mental model).
///
/// All services which descend from the same context are also said to be in the
/// same _global scope_, that is to say any service in this scope can cancel
/// _all_ other services in the same scope (including child services) at any
/// time. This is true of services in the same [ServiceGroup] for example.
///
/// # Services
///
/// - A services is said to be a _child service_ if it uses a context created
/// with `ServiceContext::branch_local`
///
/// - A service is said to be a _parent service_ if it uses a context which was
/// used to create child services.
///
/// > A parent services can always cancel all of its child services, but a child
/// > service cannot cancel its parent service.
#[derive(Default)]
pub struct ServiceContext {
    token_global: tokio_util::sync::CancellationToken,
    token_local: Option<tokio_util::sync::CancellationToken>,
    capabilities: Arc<MadaraCapabilitiesMask>,
    state: Arc<std::sync::atomic::AtomicU8>,
}

impl ServiceContext {
    pub fn new() -> Self {
        Self {
            token_global: tokio_util::sync::CancellationToken::new(),
            token_local: None,
            capabilities: Arc::new(MadaraCapabilitiesMask::default()),
            state: Arc::new(std::sync::atomic::AtomicU8::new(MadaraState::default() as u8)),
        }
    }

    /// Stops all services under the same global context scope.
    pub fn cancel_global(&self) {
        self.token_global.cancel();
    }

    /// Stops all services under the same local context scope.
    ///
    /// A local context is created by calling `branch_local` and allows you to
    /// reduce the scope of cancellation only to those services which will use
    /// the new context.
    pub fn cancel_local(&self) {
        self.token_local.as_ref().unwrap_or(&self.token_global).cancel();
    }

    pub async fn cancelled(&self) {
        if let Some(token_local) = &self.token_local {
            tokio::select! {
                _ = self.token_global.cancelled() => {},
                _ = token_local.cancelled() => {}
            }
        } else {
            tokio::select! {
                _ = self.token_global.cancelled() => {},
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.token_global.is_cancelled() || self.token_local.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
    }

    /// Copies the context, maintaining its scope.
    pub fn branch(&self) -> Self {
        Self {
            token_global: self.token_global.clone(),
            token_local: self.token_local.clone(),
            capabilities: Arc::clone(&self.capabilities),
            state: Arc::clone(&self.state),
        }
    }

    /// Copies the context into a new local scope.
    ///
    /// Any service which uses this new context will be able to cancel the
    /// services in the same local scope as itself, and any further child
    /// services, without affecting the rest of the global scope.
    pub fn child(&self) -> Self {
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global).child_token();

        Self {
            token_global: self.token_global.clone(),
            token_local: Some(token_local),
            capabilities: Arc::clone(&self.capabilities),
            state: Arc::clone(&self.state),
        }
    }

    /// Atomically checks if a set of services are running.
    ///
    /// You can combine multiple [MadaraCapability] into a single bitmask to
    /// check the state of multiple services at once.
    pub fn capability_check(&self, cap: u8) -> bool {
        self.capabilities.is_active(cap)
    }

    /// Atomically marks a service as active
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    pub fn capability_add(&mut self, cap: MadaraCapability) -> bool {
        self.capabilities.activate(cap)
    }

    /// Atomically checks the state of the node
    pub fn state(&self) -> MadaraState {
        self.state.load(std::sync::atomic::Ordering::SeqCst).into()
    }

    /// Atomically sets the state of the node
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    pub fn state_advance(&mut self) -> MadaraState {
        let state = self.state.load(std::sync::atomic::Ordering::SeqCst).saturating_add(1);
        self.state.store(state, std::sync::atomic::Ordering::SeqCst);
        state.into()
    }
}

/// The app is divided into services, with each service having a different responsability within the app.
/// Depending on the startup configuration, some services are enabled and some are disabled.
///
/// This trait enables launching nested services and groups.
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync {
    /// Default impl does not start any task.
    async fn start(&mut self, _join_set: &mut JoinSet<anyhow::Result<()>>, _ctx: ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_and_drive_to_end(mut self) -> anyhow::Result<()>
    where
        Self: Sized,
    {
        let mut join_set = JoinSet::new();
        self.start(&mut join_set, ServiceContext::new()).await.context("Starting service")?;
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
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>, ctx: ServiceContext) -> anyhow::Result<()> {
        // drive the join set as a nested task
        let mut own_join_set = self.join_set.take().expect("Service has already been started.");
        for svc in self.services.iter_mut() {
            svc.start(&mut own_join_set, ctx.child()).await.context("Starting service")?;
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
