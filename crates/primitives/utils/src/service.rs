//! Service trait and combinators.

use anyhow::Context;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt::{Debug, Display},
    panic,
    sync::Arc,
};
use tokio::task::{JoinError, JoinSet};

const SERVICE_COUNT: usize = 8;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum MadaraService {
    #[default]
    None = 0,
    Database = 1,
    L1Sync = 2,
    L2Sync = 4,
    BlockProduction = 8,
    Rpc = 16,
    RpcAdmin = 32,
    Gateway = 64,
    Telemetry = 128,
}

impl Display for MadaraService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::None => "none",
                Self::Database => "database",
                Self::L1Sync => "l1 sync",
                Self::L2Sync => "l2 sync",
                Self::BlockProduction => "block production",
                Self::Rpc => "rpc",
                Self::RpcAdmin => "rpc admin",
                Self::Gateway => "gateway",
                Self::Telemetry => "telemetry",
            }
        )
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum MadaraServiceStatus {
    On,
    Off,
}

impl Display for MadaraServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::On => "on",
                Self::Off => "off",
            }
        )
    }
}

impl std::ops::BitOr for MadaraServiceStatus {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        if self.is_on() || rhs.is_on() {
            MadaraServiceStatus::On
        } else {
            MadaraServiceStatus::Off
        }
    }
}

impl std::ops::BitAnd for MadaraServiceStatus {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        if self.is_on() && rhs.is_on() {
            MadaraServiceStatus::On
        } else {
            MadaraServiceStatus::Off
        }
    }
}

impl From<bool> for MadaraServiceStatus {
    fn from(value: bool) -> Self {
        match value {
            true => Self::On,
            false => Self::Off,
        }
    }
}

impl MadaraServiceStatus {
    pub fn is_on(&self) -> bool {
        self == &MadaraServiceStatus::On
    }

    pub fn is_off(&self) -> bool {
        self == &MadaraServiceStatus::Off
    }
}

#[repr(transparent)]
#[derive(Default)]
pub struct MadaraServiceMask(std::sync::atomic::AtomicU8);

impl MadaraServiceMask {
    #[cfg(feature = "testing")]
    pub fn new_for_testing() -> Self {
        Self(std::sync::atomic::AtomicU8::new(u8::MAX))
    }

    #[inline(always)]
    pub fn status(&self, svcs: u8) -> MadaraServiceStatus {
        (self.0.load(std::sync::atomic::Ordering::SeqCst) & svcs > 0).into()
    }

    #[inline(always)]
    pub fn is_active_some(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) != 0
    }

    #[inline(always)]
    pub fn activate(&self, svc: MadaraService) -> MadaraServiceStatus {
        let prev = self.0.fetch_or(svc as u8, std::sync::atomic::Ordering::SeqCst);
        (prev & svc as u8 > 0).into()
    }

    #[inline(always)]
    pub fn deactivate(&self, svc: MadaraService) -> MadaraServiceStatus {
        let svc = svc as u8;
        let prev = self.0.fetch_and(!svc, std::sync::atomic::Ordering::SeqCst);
        (prev & svc > 0).into()
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
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

#[derive(Clone, Copy)]
pub struct ServiceTransport {
    pub svc: MadaraService,
    pub status: MadaraServiceStatus,
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
///   with `ServiceContext::branch_local`
///
/// - A service is said to be a _parent service_ if it uses a context which was
///   used to create child services.
///
/// > A parent services can always cancel all of its child services, but a child
/// > service cannot cancel its parent service.
pub struct ServiceContext {
    token_global: tokio_util::sync::CancellationToken,
    token_local: Option<tokio_util::sync::CancellationToken>,
    services: Arc<MadaraServiceMask>,
    service_update_sender: Arc<tokio::sync::broadcast::Sender<ServiceTransport>>,
    service_update_receiver: Option<tokio::sync::broadcast::Receiver<ServiceTransport>>,
    state: Arc<std::sync::atomic::AtomicU8>,
    id: MadaraService,
}

impl Clone for ServiceContext {
    fn clone(&self) -> Self {
        Self {
            token_global: self.token_global.clone(),
            token_local: self.token_local.clone(),
            services: Arc::clone(&self.services),
            service_update_sender: Arc::clone(&self.service_update_sender),
            service_update_receiver: None,
            state: Arc::clone(&self.state),
            id: self.id,
        }
    }
}

impl Default for ServiceContext {
    fn default() -> Self {
        Self {
            token_global: tokio_util::sync::CancellationToken::new(),
            token_local: None,
            services: Arc::new(MadaraServiceMask::default()),
            service_update_sender: Arc::new(tokio::sync::broadcast::channel(SERVICE_COUNT).0),
            service_update_receiver: None,
            state: Arc::new(std::sync::atomic::AtomicU8::new(MadaraState::default() as u8)),
            id: MadaraService::default(),
        }
    }
}

impl ServiceContext {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "testing")]
    pub fn new_for_testing() -> Self {
        Self { services: Arc::new(MadaraServiceMask::new_for_testing()), ..Default::default() }
    }

    pub fn new_with_services(services: Arc<MadaraServiceMask>) -> Self {
        Self { services, ..Default::default() }
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

    /// A future which completes when the service associated to this
    /// [ServiceContext] is canceled.
    ///
    /// This happens after calling [ServiceContext::cancel_local] or
    /// [ServiceContext::cancel_global].
    ///
    /// Use this to race against other futures in a [tokio::select] for example.
    #[inline(always)]
    pub async fn cancelled(&self) {
        if self.state() != MadaraState::Shutdown {
            match &self.token_local {
                Some(token_local) => tokio::select! {
                    _ = self.token_global.cancelled() => {},
                    _ = token_local.cancelled() => {}
                },
                None => tokio::select! {
                    _ = self.token_global.cancelled() => {},
                },
            }
        }
    }

    /// Check if the service associated to this [ServiceContext] was canceled.
    ///
    /// This happens after calling [ServiceContext::cancel_local] or
    /// [ServiceContext::cancel_global].
    #[inline(always)]
    pub fn is_cancelled(&self) -> bool {
        self.token_global.is_cancelled()
            || self.token_local.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
            || self.services.status(self.id as u8) == MadaraServiceStatus::Off
            || self.state() == MadaraState::Shutdown
    }

    /// The id of service associated to this [ServiceContext]
    pub fn id(&self) -> MadaraService {
        self.id
    }

    /// Copies the context, maintaining its scope but with a new id.
    pub fn with_id(mut self, id: MadaraService) -> Self {
        self.id = id;
        self
    }

    /// Copies the context into a new local scope.
    ///
    /// Any service which uses this new context will be able to cancel the
    /// services in the same local scope as itself, and any further child
    /// services, without affecting the rest of the global scope.
    pub fn child(&self) -> Self {
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global).child_token();

        Self { token_local: Some(token_local), ..Clone::clone(self) }
    }

    /// Atomically checks if a set of services are running.
    ///
    /// You can combine multiple [MadaraService] into a single bitmask to
    /// check the state of multiple services at once.
    ///
    /// This will return [MadaraServiceStatus::On] if _any_ of the services in
    /// the bitmask are active.
    #[inline(always)]
    pub fn service_status(&self, svc: u8) -> MadaraServiceStatus {
        self.services.status(svc)
    }

    /// Atomically marks a service as active
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    #[inline(always)]
    pub fn service_add(&self, svc: MadaraService) -> MadaraServiceStatus {
        let res = self.services.activate(svc);

        // TODO: make an internal server error out of this
        let _ = self.service_update_sender.send(ServiceTransport { svc, status: MadaraServiceStatus::On });

        res
    }

    /// Atomically marks a service as inactive
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    #[inline(always)]
    pub fn service_remove(&self, svc: MadaraService) -> MadaraServiceStatus {
        let res = self.services.deactivate(svc);
        let _ = self.service_update_sender.send(ServiceTransport { svc, status: MadaraServiceStatus::Off });

        res
    }

    pub async fn service_subscribe(&mut self) -> Option<ServiceTransport> {
        if self.service_update_receiver.is_none() {
            self.service_update_receiver = Some(self.service_update_sender.subscribe());
        }

        let mut rx = self.service_update_receiver.take().expect("Receiver was set above");
        let res = tokio::select! {
            svc = rx.recv() => { svc.ok() },
            _ = self.cancelled() => { None }
        };

        self.service_update_receiver = Some(rx);
        res
    }

    /// Atomically checks if the service associated to this [ServiceContext] is
    /// active.
    ///
    /// This can be updated across threads by calling [ServiceContext::service_remove]
    /// or [ServiceContext::service_add]
    #[inline(always)]
    pub fn status(&self) -> MadaraServiceStatus {
        self.services.status(self.id as u8)
    }

    /// Atomically checks the state of the node
    #[inline(always)]
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
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        Ok(())
    }

    fn id(&self) -> MadaraService;
}

#[async_trait::async_trait]
impl Service for Box<dyn Service> {
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        self.as_mut().start(_runner).await
    }

    fn id(&self) -> MadaraService {
        self.as_ref().id()
    }
}

pub struct ServiceRunner<'a> {
    ctx: ServiceContext,
    join_set: &'a mut JoinSet<anyhow::Result<MadaraService>>,
}

impl<'a> ServiceRunner<'a> {
    fn new(ctx: ServiceContext, join_set: &'a mut JoinSet<anyhow::Result<MadaraService>>) -> Self {
        Self { ctx, join_set }
    }

    pub fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    pub fn start_service<F, E>(self, runner: impl FnOnce(ServiceContext) -> F + Send + 'static)
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<anyhow::Error> + Send,
    {
        let Self { ctx, join_set } = self;
        join_set.spawn(async move {
            let id = ctx.id();
            runner(ctx).await.map_err(Into::into)?;
            Ok(id)
        });
    }
}

pub struct ServiceMonitor {
    services: [Option<Box<dyn Service>>; SERVICE_COUNT],
    join_set: JoinSet<anyhow::Result<MadaraService>>,
    service_started: Arc<MadaraServiceMask>,
    service_stopped: Arc<MadaraServiceMask>,
}

impl Default for ServiceMonitor {
    fn default() -> Self {
        Self {
            services: Default::default(),
            join_set: Default::default(),
            service_started: Default::default(),
            service_stopped: Arc::new(MadaraServiceMask(std::sync::atomic::AtomicU8::new(u8::MAX))),
        }
    }
}

impl ServiceMonitor {
    pub fn with(mut self, svc: impl Service) -> anyhow::Result<Self> {
        let idx = (svc.id() as u8).to_be().leading_zeros() as usize;
        self.services[idx] = match self.services[idx] {
            Some(_) => anyhow::bail!("Services has already been added"),
            None => Some(Box::new(svc)),
        };

        anyhow::Ok(self)
    }

    pub fn activate(self, id: MadaraService) -> Self {
        self.service_started.activate(id);
        self
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        let mut ctx = ServiceContext::new_with_services(Arc::clone(&self.service_started));

        for svc in self.services.iter_mut() {
            match svc {
                Some(svc) if self.service_started.status(svc.id() as u8) == MadaraServiceStatus::On => {
                    let runner = ServiceRunner::new(ctx.child().with_id(svc.id()), &mut self.join_set);
                    svc.start(runner).await.context("Starting service")?;
                }
                _ => continue,
            }
        }

        while self.service_started.is_active_some() {
            tokio::select! {
                Some(result) = self.join_set.join_next() => {
                    match result {
                        Ok(result) => {
                            let id = result?;
                            self.service_stopped.deactivate(id);
                        }
                        Err(panic_error) if panic_error.is_panic() => {
                            // bubble up panics too
                            panic::resume_unwind(panic_error.into_panic());
                        }
                        Err(_task_cancelled_error) => {}
                    }
                },
                Some(ServiceTransport { svc, status }) = ctx.service_subscribe() => {
                    if status == MadaraServiceStatus::On {
                        if let Some(svc) = self.services[svc as usize].as_mut() {
                            let runner = ServiceRunner::new(ctx.child().with_id(svc.id()), &mut self.join_set);
                            svc.start(runner)
                                .await
                                .context("Starting service")?;
                        }
                    }
                },
            };
        }

        Ok(())
    }
}
