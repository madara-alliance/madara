//! Madara Services Architecture
//!
//! Madara follows a [microservice](microservices) architecture to simplify the
//! composability and parallelism of its services. That is to say services can
//! be started in different orders, at different points in the program's
//! execution, stopped and even restarted. The advantage in parallelism arises
//! from the fact that each services runs as its own non-blocking asynchronous
//! task which allows for high throughput. Inter-service communication is done
//! via [tokio::sync] or more often through direct database reads and writes.
//!
//! ---
//!
//! # The [Service] trait
//!
//! This is the backbone of Madara services and serves as a common interface to
//! all. The [Service] trait specifies how a service must start as well as how
//! to _identify_ it. For reasons of atomicity, services are currently
//! identified by a single [std::sync::atomic::AtomicU8]. More about this later.
//!
//! Services are started with [Service::start] using [ServiceRunner::service_loop].
//! [ServiceRunner::service_loop] is a function which takes in a future: this
//! future represents the main loop of your service, and should run until your
//! service completes or is canceled.
//!
//! It is part of the contract of the [Service] trait that calls to
//! [ServiceRunner::service_loop] should not complete until the service has
//! _finished_ execution (this should be evident by the name) as this is used
//! to mark a service as complete and therefore ready to restart. Services where
//! [ServiceRunner::service_loop] completes _before_ the service has finished
//! execution will be automatically marked for shutdown as a safety mechanism.
//! This is done as a safeguard to avoid an invalid state where it would be
//! impossible for the node to shutdown.
//!
//! > **Note**
//! > It is assumed that services can and might be restarted. You have the
//! > responsibility to ensure this is possible. This means you should make sure
//! > not to use the like of [std::mem::take] or similar on your service inside
//! > [Service::start]. In general, make sure your service still contains all
//! > the necessary information it needs to restart. This might mean certain
//! > attributes need to be stored as a [std::sync::Arc] and cloned so that the
//! > future in [ServiceRunner::service_loop] can safely take ownership of them.
//!
//! ## An incorrect implementation of the [Service] trait
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceRunner;
//! # use mp_utils::service::MadaraServiceId;
//!
//! pub struct MyService;
//!
//! #[async_trait::async_trait]
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |ctx| async {
//!             tokio::task::spawn(async {
//!                 tokio::time::sleep(std::time::Duration::MAX).await;
//!             });
//!
//!             // This is incorrect, as the future passed to service_loop will
//!             // resolve before the task spawned above completes, meaning
//!             // Madara will incorrectly mark this service as ready to restart.
//!             // In a more complex scenario, this means we might enter an
//!             // invalid state!
//!             anyhow::Ok(())
//!         });
//!
//!         anyhow::Ok(())
//!     }
//!
//!     fn id(&self) -> MadaraServiceId {
//!         MadaraServiceId::Monitor
//!     }
//! }
//! ```
//!
//! ## A correct implementation of the [Service] trait
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceRunner;
//! # use mp_utils::service::MadaraServiceId;
//!
//! pub struct MyService;
//!
//! #[async_trait::async_trait]
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |ctx| async {
//!             tokio::time::sleep(std::time::Duration::MAX).await;
//!
//!             // This is correct, as the future passed to service_loop will
//!             // only resolve once the task above completes, so Madara can
//!             // correctly mark this service as ready to restart.
//!             anyhow::Ok(())
//!         });
//!
//!         anyhow::Ok(())
//!     }
//!
//!     fn id(&self) -> MadaraServiceId {
//!         MadaraServiceId::Monitor
//!     }
//! }
//! ```
//!
//! Or if you really need to spawn a background task:
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceRunner;
//! # use mp_utils::service::MadaraServiceId;
//!
//! pub struct MyService;
//!
//! #[async_trait::async_trait]
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |mut ctx| async move {
//!             let mut ctx1 = ctx.clone();
//!             tokio::task::spawn(async move {
//!                 tokio::select! {
//!                     _ = tokio::time::sleep(std::time::Duration::MAX) => {},
//!                     _ = ctx1.cancelled() => {},
//!                 }
//!             });
//!
//!             ctx.cancelled().await;
//!
//!             // This is correct, as even though we are spawning a background
//!             // task we have implemented a cancellation mechanism with ctx
//!             // and are waiting for that cancellation in service_loop.
//!             anyhow::Ok(())
//!         });
//!
//!         anyhow::Ok(())
//!     }
//!
//!     fn id(&self) -> MadaraServiceId {
//!         MadaraServiceId::Monitor
//!     }
//! }
//! ```
//!
//! This sort of problem generally arises in cases similar to the above, where
//! the service's role is to spawn another background task. This is can happen
//! when the service needs to start a server for example. Either avoid spawning
//! a detached task or use mechanisms such as [ServiceContext::cancelled] to
//! await for the service's completion.
//!
//! Note that by design service shutdown is designed to be manual. We still
//! implement a [SERVICE_GRACE_PERIOD] which is the maximum duration a service
//! is allowed to take to shutdown, after which it is forcefully canceled. This
//! should not happen in practice and only serves to avoid cases where someone
//! would forgets to implement a cancellation check. More on this in the next
//! section.
//!
//! ---
//!
//! # Cancellation status and inter-process requests
//!
//! Services are passed a [ServiceContext] as part of [ServiceRunner::service_loop]
//! to be used during their execution to check for and request cancellation.
//! Services can also start child services with [ServiceContext::child] to
//! create a hierarchy of services.
//!
//! ## Cancellation checks
//!
//! The main advantage of [ServiceContext] is that it allows you to gracefully
//! handle the shutdown of your services by checking for cancellation at logical
//! points in the execution, such as every iteration of a service's main loop.
//! You can use the following methods to check for cancellation, each with their
//! own caveats.
//!
//! - [ServiceContext::is_cancelled]: synchronous, useful in non-blocking
//!   scenarios.
//! - [ServiceContext::cancelled]: a future which resolves upon service
//!   cancellation. Useful to wait on a service or alongside [tokio::select].
//!
//! > **Warning**
//! > It is your responsibility to check for cancellation inside of your
//! > service. If you do not, or your service takes longer than
//! > [SERVICE_GRACE_PERIOD] to shutdown, then your service will be forcefully
//! > canceled.
//!
//! ## Cancellation requests
//!
//! Any service with access to a [ServiceContext] can request the cancellation
//! of _any other service, at any point during execution_. This can be used for
//! error handling for example, by having a single service shut itself down
//! without affecting other services, or for administrative and testing purposes
//! by having a node operator toggle services on and off from a remote endpoint.
//!
//! You can use the following methods to request for the cancellation of a
//! service:
//!
//! - [ServiceContext::cancel_global]: cancels all services.
//! - [ServiceContext::cancel_local]: cancels this service and all its children.
//! - [ServiceContext::service_remove]: cancel a specific service.
//!
//! ## Start requests
//!
//! You can _request_ for a service to be restarted by calling
//! [ServiceContext::service_add]. This is not guaranteed to work, and will fail
//! if the service is already running or if it has not been registered to
//! [the set of global services](#service-orchestration) at the start of the
//! program.
//!
//! ## Atomic status checks
//!
//! All service updates and checks are performed atomically with the use of
//! [tokio_util::sync::CancellationToken] and [MadaraServiceMask], which is a
//! [std::sync::atomic::AtomicU8] bitmask with strong [std::sync::atomic::Ordering::SeqCst]
//! cross-thread ordering of operations. Services are represented as unique
//! powers of 2 with [MadaraServiceId]. You can use this to construct your own
//! mask to check the status of multiple related services at once using
//! [MadaraServiceMask::status].
//!
//! > **Note**
//! > The use of [std::sync::atomic::AtomicU8] limits the number of possible
//! > services to 8. This might be increased in the future if new services are
//! > added.
//!
//! ---
//!
//! # Service orchestration
//!
//! Services are orchestrated by a [ServiceMonitor], which is responsible for
//! registering services, marking them as active or inactive as well as starting
//! and restarting them upon request. [ServiceMonitor] also handles the
//! cancellation of all services upon receiving a `SIGINT` or `SIGTERM`.
//!
//! > **Important**
//! > Services cannot be started or restarted if they have not been registered
//! > with [ServiceMonitor::with].
//!
//! Services are run to completion until no service remains, at which point the
//! node will automatically shutdown.
//!
//! [microservices]: https://en.wikipedia.org/wiki/Microservices

use anyhow::Context;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    panic,
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinSet;

pub const SERVICE_COUNT: usize = 8;
pub const SERVICE_GRACE_PERIOD: Duration = Duration::from_secs(10);

/// Represents a [Service] as a unique power of 2.
///
/// Note that 0 represents [MadaraServiceId::Monitor] as [ServiceMonitor] is
/// always running and therefore is the genesis state of all other services.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MadaraServiceId {
    #[default]
    #[serde(skip)]
    Monitor = 0,
    #[serde(skip)]
    Database = 1,
    L1Sync = 2,
    L2Sync = 4,
    BlockProduction = 8,
    #[serde(rename = "rpc")]
    RpcUser = 16,
    #[serde(skip)]
    RpcAdmin = 32,
    Gateway = 64,
    Telemetry = 128,
}

impl Display for MadaraServiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Monitor => "monitor",
                Self::Database => "database",
                Self::L1Sync => "l1 sync",
                Self::L2Sync => "l2 sync",
                Self::BlockProduction => "block production",
                Self::RpcUser => "rpc user",
                Self::RpcAdmin => "rpc admin",
                Self::Gateway => "gateway",
                Self::Telemetry => "telemetry",
            }
        )
    }
}

impl std::ops::BitOr for MadaraServiceId {
    type Output = u8;

    fn bitor(self, rhs: Self) -> Self::Output {
        self as u8 | rhs as u8
    }
}

impl std::ops::BitAnd for MadaraServiceId {
    type Output = u8;

    fn bitand(self, rhs: Self) -> Self::Output {
        self as u8 & rhs as u8
    }
}

impl From<u8> for MadaraServiceId {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Monitor,
            1 => Self::Database,
            2 => Self::L1Sync,
            4 => Self::L2Sync,
            8 => Self::BlockProduction,
            16 => Self::RpcUser,
            32 => Self::RpcAdmin,
            64 => Self::Gateway,
            _ => Self::Telemetry,
        }
    }
}

impl MadaraServiceId {
    /// Converts a [MadaraServiceId] into a unique index which can be used in an
    /// array.
    pub fn index(&self) -> usize {
        (*self as u8).to_be().leading_zeros() as usize
    }
}

// A boolean status enum, for clarity's sake
#[derive(PartialEq, Eq, Clone, Copy, Default, Serialize, Deserialize)]
pub enum MadaraServiceStatus {
    On,
    #[default]
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

impl std::ops::BitOr for &MadaraServiceStatus {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        if self.is_on() || rhs.is_on() {
            &MadaraServiceStatus::On
        } else {
            &MadaraServiceStatus::Off
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

impl std::ops::BitAnd for &MadaraServiceStatus {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        if self.is_on() && rhs.is_on() {
            &MadaraServiceStatus::On
        } else {
            &MadaraServiceStatus::Off
        }
    }
}

impl std::ops::BitOrAssign for MadaraServiceStatus {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = if self.is_on() || rhs.is_on() { MadaraServiceStatus::On } else { MadaraServiceStatus::Off }
    }
}

impl std::ops::BitAndAssign for MadaraServiceStatus {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = if self.is_on() && rhs.is_on() { MadaraServiceStatus::On } else { MadaraServiceStatus::Off }
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
    #[inline(always)]
    pub fn is_on(&self) -> bool {
        self == &MadaraServiceStatus::On
    }

    #[inline(always)]
    pub fn is_off(&self) -> bool {
        self == &MadaraServiceStatus::Off
    }
}

/// An atomic bitmask of each [MadaraServiceId]'s status with strong
/// [std::sync::atomic::Ordering::SeqCst] cross-thread ordering of operations.
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
        self.0.load(std::sync::atomic::Ordering::SeqCst) > 0
    }

    #[inline(always)]
    pub fn activate(&self, svc: MadaraServiceId) -> MadaraServiceStatus {
        let prev = self.0.fetch_or(svc as u8, std::sync::atomic::Ordering::SeqCst);
        (prev & svc as u8 > 0).into()
    }

    #[inline(always)]
    pub fn deactivate(&self, svc: MadaraServiceId) -> MadaraServiceStatus {
        let svc = svc as u8;
        let prev = self.0.fetch_and(!svc, std::sync::atomic::Ordering::SeqCst);
        (prev & svc > 0).into()
    }

    fn active_set(&self) -> Vec<MadaraServiceId> {
        let mut i = MadaraServiceId::Telemetry as u8;
        let state = self.0.load(std::sync::atomic::Ordering::SeqCst);
        let mut set = Vec::with_capacity(SERVICE_COUNT);

        while i > 0 {
            let mask = i & state;

            if mask > 0 {
                set.push(MadaraServiceId::from(mask));
            }

            i >>= 1;
        }

        set
    }
}

/// Atomic state and cancellation context associated to a [Service].
///
/// # Scope
///
/// You can create a hierarchy of services by calling [ServiceContext::child].
/// Services are said to be in the same _local scope_ if they inherit the same
/// `token_local` [tokio_util::sync::CancellationToken]. You can think of
/// services being local if they can cancel each other without affecting the
/// rest of the app.
///
/// All services which are derived from the same [ServiceContext] are said to
/// be in the same _global scope_, that is to say any service in this scope can
/// cancel _all_ other services in the same scope (including child services) at
/// any time. This is true of services in the same [ServiceMonitor] for example.
///
/// # Service hierarchy
///
/// - A services is said to be a _child service_ if it uses a context created
///   with [ServiceContext::child]
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
    id: MadaraServiceId,
}

impl Clone for ServiceContext {
    fn clone(&self) -> Self {
        Self {
            token_global: self.token_global.clone(),
            token_local: self.token_local.clone(),
            services: Arc::clone(&self.services),
            service_update_sender: Arc::clone(&self.service_update_sender),
            service_update_receiver: None,
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
            id: MadaraServiceId::Monitor,
        }
    }
}

impl ServiceContext {
    /// Creates a new [Default] [ServiceContext]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "testing")]
    pub fn new_for_testing() -> Self {
        Self {
            services: Arc::new(MadaraServiceMask::new_for_testing()),
            id: MadaraServiceId::Database,
            ..Default::default()
        }
    }

    /// Creates a new [Default] [ServiceContext] with the state of its services
    /// set to the specified value.
    pub fn new_with_services(services: Arc<MadaraServiceMask>) -> Self {
        Self { services, ..Default::default() }
    }

    /// Stops all services under the same global context scope.
    pub fn cancel_global(&self) {
        tracing::info!("🔌 Gracefully shutting down node");

        self.token_global.cancel();
    }

    /// Stops all services under the same local context scope.
    ///
    /// A local context is created by calling [ServiceContext::child] and allows
    /// you to reduce the scope of cancellation only to those services which
    /// will use the new context.
    pub fn cancel_local(&self) {
        self.token_local.as_ref().unwrap_or(&self.token_global).cancel();
    }

    /// A future which completes when the service associated to this
    /// [ServiceContext] is canceled.
    ///
    /// A service is canceled after calling [ServiceContext::cancel_local],
    /// [ServiceContext::cancel_global] or if it is marked for removal with
    /// [ServiceContext::service_remove].
    ///
    /// Use this to race against other futures in a [tokio::select] or keep a
    /// coroutine alive for as long as the service itself.
    #[inline(always)]
    pub async fn cancelled(&mut self) {
        if self.service_update_receiver.is_none() {
            self.service_update_receiver = Some(self.service_update_sender.subscribe());
        }

        let mut rx = self.service_update_receiver.take().expect("Receiver was set above");
        let token_global = &self.token_global;
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global);

        loop {
            // We keep checking for service status updates until a token has
            // been canceled or this service was deactivated
            let res = tokio::select! {
                svc = rx.recv() => svc.ok(),
                _ = token_global.cancelled() => break,
                _ = token_local.cancelled() => break
            };

            if let Some(ServiceTransport { svc_id, status }) = res {
                if svc_id == self.id && status == MadaraServiceStatus::Off {
                    return;
                }
            }
        }
    }

    /// Checks if the service associated to this [ServiceContext] was canceled.
    ///
    /// This happens after calling [ServiceContext::cancel_local],
    /// [ServiceContext::cancel_global].or [ServiceContext::service_remove].
    ///
    /// # Limitations
    ///
    /// This function should _not_ be used when waiting on potentially
    /// blocking futures which can be canceled without entering an invalid
    /// state. The latter is important, so let's break this down.
    ///
    /// - _blocking future_: this is blocking at a [Service] level, not at the
    ///   node level. A blocking task in this sense in a task which prevents a
    ///   service from making progress in its execution, but not necessarily the
    ///   rest of the node. A prime example of this is when you are waiting on
    ///   a channel, and updates to that channel are sparse, or even unique.
    ///
    /// - _entering an invalid state_: the entire point of [ServiceContext] is
    ///   to allow services to gracefully shutdown. We do not want to be, for
    ///   example, racing each service against a global cancellation future, as
    ///   not every service might be cancellation safe (we still do this
    ///   somewhat with [SERVICE_GRACE_PERIOD] but this is a last resort and
    ///   should not execute in normal circumstances). Put differently, we do
    ///   not want to stop in the middle of a critical computation before it has
    ///   been saved to disk.
    ///
    /// Putting this together, [ServiceContext::is_cancelled] is only suitable
    /// for checking cancellation alongside tasks which will not block the
    /// running service, or in very specific circumstances where waiting on a
    /// blocking future has higher precedence than shutting down the node.
    ///
    /// Examples of when to use [ServiceContext::is_cancelled]:
    ///
    /// - All your computation does is sleep or tick away a short period of
    ///   time.
    /// - You are checking for cancellation inside of synchronous code.
    ///
    /// If this does not describe your usage, and you are waiting on a blocking
    /// future, which is cancel-safe and which does not risk putting the node
    /// in an invalid state if canceled, then you should be using
    /// [ServiceContext::cancelled] instead.
    #[inline(always)]
    pub fn is_cancelled(&self) -> bool {
        self.token_global.is_cancelled()
            || self.token_local.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
            || self.services.status(self.id as u8) == MadaraServiceStatus::Off
    }

    /// The id of the [Service] associated to this [ServiceContext]
    pub fn id(&self) -> MadaraServiceId {
        self.id
    }

    /// Sets the id of this [ServiceContext]
    pub fn with_id(mut self, id: MadaraServiceId) -> Self {
        self.id = id;
        self
    }

    /// Creates a new [ServiceContext] as a child of the current context.
    ///
    /// Any [Service] which uses this new context will be able to cancel the
    /// services in the same local scope as itself, and any further child
    /// services, without affecting the rest of the global scope.
    pub fn child(&self) -> Self {
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global).child_token();

        Self { token_local: Some(token_local), ..Clone::clone(self) }
    }

    /// Atomically checks if a set of [Service]s are running.
    ///
    /// You can combine multiple [MadaraServiceId]s into a single bitmask to
    /// check the state of multiple services at once. This will return
    /// [MadaraServiceStatus::On] if _any_ of the services in the bitmask are
    /// active.
    #[inline(always)]
    pub fn service_status(&self, svc: u8) -> MadaraServiceStatus {
        self.services.status(svc)
    }

    /// Atomically marks a [Service] as active.
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    ///
    /// You can use [ServiceContext::service_subscribe] to subscribe to changes
    /// in the status of any service.
    #[inline(always)]
    pub fn service_add(&self, svc_id: MadaraServiceId) -> MadaraServiceStatus {
        let res = self.services.activate(svc_id);

        // TODO: make an internal server error out of this
        let _ = self.service_update_sender.send(ServiceTransport { svc_id, status: MadaraServiceStatus::On });

        res
    }

    /// Atomically marks a [Service] as inactive.
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    ///
    /// You can use [ServiceContext::service_subscribe] to subscribe to changes
    /// in the status of any service.
    #[inline(always)]
    pub fn service_remove(&self, svc_id: MadaraServiceId) -> MadaraServiceStatus {
        let res = self.services.deactivate(svc_id);
        let _ = self.service_update_sender.send(ServiceTransport { svc_id, status: MadaraServiceStatus::Off });

        res
    }

    /// Opens up a new subscription which will complete once the status of a
    /// [Service] has been updated.
    ///
    /// This subscription is stored on first call to this method and can be
    /// accessed through the same instance of [ServiceContext].
    ///
    /// # Returns
    ///
    /// Identifying information about the service which was updated as well
    /// as its new [MadaraServiceStatus].
    pub async fn service_subscribe(&mut self) -> Option<ServiceTransport> {
        if self.service_update_receiver.is_none() {
            self.service_update_receiver = Some(self.service_update_sender.subscribe());
        }

        let mut rx = self.service_update_receiver.take().expect("Receiver was set above");
        let token_global = &self.token_global;
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global);

        let res = tokio::select! {
            svc = rx.recv() => svc.ok(),
            _ = token_global.cancelled() => None,
            _ = token_local.cancelled() => None
        };

        // ownership hack: `rx` cannot depend on a mutable borrow to `self` as we
        // also depend on immutable borrows for `token_local` and `token_global`
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
}

/// Provides info about a [Service]'s status.
///
/// Used as part of [ServiceContext::service_subscribe].
#[derive(Clone, Copy)]
pub struct ServiceTransport {
    pub svc_id: MadaraServiceId,
    pub status: MadaraServiceStatus,
}

/// A microservice in the Madara node.
///
/// The app is divided into services, with each service handling different
/// responsibilities within the app. Depending on the startup configuration,
/// some services are enabled and some are disabled.
///
/// Services should be started with [ServiceRunner::service_loop].
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync {
    /// Default impl does not start any task.
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        Ok(())
    }

    fn id(&self) -> MadaraServiceId;
}

#[async_trait::async_trait]
impl Service for Box<dyn Service> {
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        self.as_mut().start(_runner).await
    }

    fn id(&self) -> MadaraServiceId {
        self.as_ref().id()
    }
}

/// Wrapper around a [tokio::task::JoinSet] and a [ServiceContext].
///
/// Used to enforce certain shutdown behavior onto [Service]s which are started
/// with [ServiceRunner::service_loop]
pub struct ServiceRunner<'a> {
    ctx: ServiceContext,
    join_set: &'a mut JoinSet<anyhow::Result<MadaraServiceId>>,
}

impl<'a> ServiceRunner<'a> {
    fn new(ctx: ServiceContext, join_set: &'a mut JoinSet<anyhow::Result<MadaraServiceId>>) -> Self {
        Self { ctx, join_set }
    }

    /// The main loop of a [Service].
    ///
    /// The future passed to this function should complete _only once the
    /// service completes or is canceled_. Services that complete early will
    /// automatically be canceled.
    ///
    /// > **Caution**
    /// > As a safety mechanism, services have up to [SERVICE_GRACE_PERIOD]
    /// > to gracefully shutdown before they are forcefully canceled. This
    /// > should not execute in a normal context and only serves to prevent
    /// > infinite loops on shutdown request if services have not been
    /// > implemented correctly.
    #[tracing::instrument(skip(self, runner), fields(module = "Service"))]
    pub fn service_loop<F, E>(self, runner: impl FnOnce(ServiceContext) -> F + Send + 'static)
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<anyhow::Error> + Send,
    {
        let Self { ctx, join_set } = self;
        join_set.spawn(async move {
            let id = ctx.id();
            if id != MadaraServiceId::Monitor {
                tracing::debug!("Starting {id}");
            }

            // If a service is implemented correctly, `stopper` should never
            // cancel first. This is a safety measure in case someone forgets to
            // implement a cancellation check along some branch of the service's
            // execution, or if they don't read the docs :D
            let ctx1 = ctx.clone();
            tokio::select! {
                res = runner(ctx) => res.map_err(Into::into)?,
                _ = Self::stopper(ctx1, &id) => {},
            }

            if id != MadaraServiceId::Monitor {
                tracing::debug!("Shutting down {id}");
            }

            Ok(id)
        });
    }

    async fn stopper(mut ctx: ServiceContext, id: &MadaraServiceId) {
        ctx.cancelled().await;
        tokio::time::sleep(SERVICE_GRACE_PERIOD).await;

        tracing::warn!("⚠️  Forcefully shutting down {id}");
    }
}

#[derive(Default)]
pub struct ServiceMonitor {
    services: [Option<Box<dyn Service>>; SERVICE_COUNT],
    join_set: JoinSet<anyhow::Result<MadaraServiceId>>,
    status_request: Arc<MadaraServiceMask>,
    status_actual: Arc<MadaraServiceMask>,
}

/// Orchestrates the execution of various [Service]s.
///
/// A [ServiceMonitor] is responsible for registering services, starting and
/// stopping them as well as handling `SIGINT` and `SIGTERM`. Services are run
/// to completion until no service remains, at which point the node will
/// automatically shutdown.
///
/// All services are inactive by default. Only the services which are marked as
/// _explicitly active_ with [ServiceMonitor::activate] will be automatically
/// started when calling [ServiceMonitor::start]. If no service was activated,
/// the node will shutdown.
///
/// Note that services which are not added with [ServiceMonitor::with] cannot
/// be started or restarted.
impl ServiceMonitor {
    /// Registers a [Service] to the [ServiceMonitor]. This service is
    /// _inactive_ by default and can be started at a later time.
    pub fn with(mut self, svc: impl Service) -> anyhow::Result<Self> {
        let idx = svc.id().index();
        self.services[idx] = match self.services[idx] {
            Some(_) => anyhow::bail!("Services has already been added"),
            None => Some(Box::new(svc)),
        };

        anyhow::Ok(self)
    }

    /// Marks a [Service] as active, meaning it will be started automatically
    /// when calling [ServiceMonitor::start].
    pub fn activate(&self, id: MadaraServiceId) {
        self.status_request.activate(id);
    }

    /// Starts all activate [Service]s and runs them to completion. Services
    /// are activated by calling [ServiceMonitor::activate]. This function
    /// completes once all services have been run to completion.
    ///
    /// Keep in mind that services can be restarted as long as other services
    /// are running (otherwise the node would shutdown).
    #[tracing::instrument(skip(self), fields(module = "Service"))]
    pub async fn start(mut self) -> anyhow::Result<()> {
        let mut ctx = ServiceContext::new_with_services(Arc::clone(&self.status_request));

        // start only the initially active services
        for svc in self.services.iter_mut() {
            match svc {
                Some(svc) if self.status_request.status(svc.id() as u8) == MadaraServiceStatus::On => {
                    let id = svc.id();
                    self.status_actual.activate(id);

                    let ctx = ctx.child().with_id(id);
                    let runner = ServiceRunner::new(ctx, &mut self.join_set);
                    svc.start(runner).await.context("Starting service")?;
                }
                _ => continue,
            }
        }

        // SIGINT & SIGTERM
        let runner = ServiceRunner::new(ctx.clone(), &mut self.join_set);
        runner.service_loop(|ctx| async move {
            let sigint = tokio::signal::ctrl_c();
            let sigterm = async {
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(mut signal) => signal.recv().await,
                    Err(_) => core::future::pending().await, // SIGTERM not supported
                }
            };

            tokio::select! {
                res = sigint => res?,
                _ = sigterm => {},
            };

            ctx.cancel_global();

            anyhow::Ok(())
        });

        tracing::debug!("Running services: {:?}", self.status_request.active_set());
        while self.status_request.is_active_some() {
            tokio::select! {
                // A service has run to completion, mark it as inactive
                Some(result) = self.join_set.join_next() => {
                    match result {
                        Ok(result) => {
                            let id = result?;
                            tracing::debug!("service {id} has shut down");
                            self.status_actual.deactivate(id);
                            self.status_request.deactivate(id);
                        }
                        Err(panic_error) if panic_error.is_panic() => {
                            // bubble up panics too
                            panic::resume_unwind(panic_error.into_panic());
                        }
                        Err(_task_cancelled_error) => {}
                    }
                },
                // A service has had its status updated, check if it is a
                // restart request
                Some(ServiceTransport { svc_id, status }) = ctx.service_subscribe() => {
                    if status == MadaraServiceStatus::On {
                        if let Some(svc) = self.services[svc_id.index()].as_mut() {
                            if self.status_actual.status(svc_id as u8) == MadaraServiceStatus::Off {
                                self.status_actual.activate(svc_id);

                                let ctx = ctx.child().with_id(svc_id);
                                let runner = ServiceRunner::new(ctx, &mut self.join_set);
                                svc.start(runner)
                                    .await
                                    .context("Starting service")?;

                                tracing::debug!("service {svc_id} has started");
                            } else {
                                // reset request
                                self.status_request.deactivate(svc_id);
                            }
                        }
                    }
                },
                else => continue
            };

            tracing::debug!("Services still active: {:?}", self.status_request.active_set());
        }

        Ok(())
    }
}
