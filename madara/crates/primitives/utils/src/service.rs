//! # Madara Services Architecture
//!
//! Madara follows a [`microservice`] architecture to simplify the composability and parallelism of
//! its services. That is to say services can be started in different orders, at different points in
//! the program's execution, stopped and even restarted. The advantage in parallelism arises from
//! the fact that each services runs as its own non-blocking asynchronous task which allows for high
//! throughput. Inter-service communication is done via [`tokio::sync`] or more often through direct
//! database reads and writes.
//!
//! Services are run to completion until no service remains, at which point the
//! node will automatically shutdown.
//!
//! # The [`Service`] trait
//!
//! This is the backbone of Madara services. The [`Service`] trait specifies how a service must start.
//! To be identified, a [`Service`] must also implement [`ServiceIdProvider`] (this is a separate trait
//! for reasons related to boxing). Services can be identified by any type which implements
//! [`ServiceId`].
//!
//! Under the hood, services are identified using [`String`]s, but this is wrapped around the
//! [`ServiceId`] trait to provide type safety.
//!
//! Services are started from [`Service::start`] using [`ServiceRunner::service_loop`]. [`service_loop`]
//! is a function which takes in a future: this is the main loop of a service, and should run until
//! the service completes or is [`cancelled`].
//!
//! It is part of the contract of the [`Service`] trait that calls to [`service_loop`] **should not
//! complete** until the service has _finished_ execution as this is used to mark a service as
//! ready to restart. This means your service should keep yielding [`Poll::Pending`] until it is done.
//! Services where [`service_loop`] completes _before_ the service has finished execution will be
//! automatically marked for shutdown. This is done to avoid an invalid state. This means you should
//! not run the work your service does inside a [`tokio::task::spawn`] and exit [`service_loop`]
//! immediately for example. For running blocking futures inside your service, refer to
//! [`ServiceContext::run_until_cancelled`].
//!
//! It is assumed that services can and might be restarted. You have the responsibility to ensure
//! this is possible. This means you should make sure not to use the likes of [`std::mem::take`] or
//! similar inside [`Service::start`]. In general, make sure your service still contains all the
//! necessary information it needs to restart. This might mean certain attributes need to be
//! stored as a [`std::sync::Arc`] and cloned so that the future in [`service_loop`] can safely take
//! ownership of them.
//!
//! ## An incorrect implementation of the [`Service`] trait
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::ServiceIdProvider;
//! # use mp_utils::service::ServiceRunner;
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
//!             // resolve before the task spawned above completes, meaning the
//!             // service monitor will incorrectly mark this service as ready
//!             // to restart. In a more complex scenario, this means we might
//!             // enter an invalid state!
//!             anyhow::Ok(())
//!         })
//!     }
//! }
//!
//! impl ServiceIdProvider for MyService {
//!     fn id_provider(&self) -> impl ServiceId {
//!         MyServiceId
//!     }
//! }
//!
//! pub struct MyServiceId;
//!
//! impl ServiceId for MyServiceId {
//!     fn svc_id(&self) -> String {
//!         "MyService".to_string()
//!     }
//! }
//! ```
//!
//! ## A correct implementation of the [`Service`] trait
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::ServiceIdProvider;
//! # use mp_utils::service::ServiceRunner;
//! pub struct MyService;
//!
//! #[async_trait::async_trait]
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |mut ctx| async move {
//!             ctx.run_until_cancelled(tokio::time::sleep(std::time::Duration::MAX)).await;
//!
//!             // This is correct, as the future passed to service_loop will
//!             // only resolve once the task above completes, so the service
//!             // monitor can correctly mark this service as ready to restart.
//!             anyhow::Ok(())
//!         })
//!     }
//! }
//!
//! impl ServiceIdProvider for MyService {
//!     fn id_provider(&self) -> impl ServiceId {
//!         MyServiceId
//!     }
//! }
//!
//! pub struct MyServiceId;
//!
//! impl ServiceId for MyServiceId {
//!     fn svc_id(&self) -> String {
//!         "MyService".to_string()
//!     }
//! }
//! ```
//!
//! Or if you really need to spawn a background task:
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::ServiceIdProvider;
//! # use mp_utils::service::ServiceRunner;
//! pub struct MyService;
//!
//! #[async_trait::async_trait]
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |mut ctx| async move {
//!             let mut ctx1 = ctx.clone();
//!             tokio::task::spawn(async move {
//!                 ctx1.run_until_cancelled(tokio::time::sleep(std::time::Duration::MAX)).await;
//!             });
//!
//!             ctx.cancelled().await;
//!
//!             // This is correct, as even though we are spawning a background
//!             // task we have implemented a cancellation mechanism with the
//!             // service context and are waiting for that cancellation in the
//!             // service loop.
//!             anyhow::Ok(())
//!         })
//!     }
//! }
//!
//! impl ServiceIdProvider for MyService {
//!     fn id_provider(&self) -> impl ServiceId {
//!         MyServiceId
//!     }
//! }
//!
//! pub struct MyServiceId;
//!
//! impl ServiceId for MyServiceId {
//!     fn svc_id(&self) -> String {
//!         "MyService".to_string()
//!     }
//! }
//! ```
//!
//! This sort of problem generally arises in cases where the service's role is to spawn another
//! background task (when starting a server for example). Either avoid spawning a detached task or
//! use mechanisms such as [`cancelled`] to await for the service's completion.
//!
//! Note that service shutdown is designed to be manual, ie: services should never be forcefully
//! shutdown unless you forget to implement proper [`cancellation`]. To avoid this edge case, we still
//! implement a [`SERVICE_GRACE_PERIOD`] which is the maximum duration a service is allowed to take to
//! shutdown, after which it is forcefully cancelled. This should not happen in practice and only
//! serves to avoid cases where someone would forget to implement a cancellation check.
//!
//! ---
//!
//! # Cancellation status and inter-process requests
//!
//! Services are passed a [`ServiceContext`] as part of [`service_loop`] to be used during their
//! execution to check for and request cancellation. Services can also start child services with
//! [`ServiceContext::child`] to create a hierarchy of services.
//!
//! ## Cancellation checks
//!
//! [`ServiceContext`] allows you to gracefully handle the shutting down services by manually checking
//! for  cancellation at logical points in the execution. You can use the following methods to check
//! for cancellation.
//!
//! - [`is_cancelled`]: synchronous, useful in non-blocking scenarios.
//! - [`cancelled`]: a future which resolves upon service cancellation. Useful to wait on a service or
//!   alongside [`tokio::select`].
//!
//! <div class="warning">
//!
//! It is your responsibility to check for cancellation inside of your service. If you do not, or
//! your service takes longer than [`SERVICE_GRACE_PERIOD`] to shutdown, then your service will be
//! forcefully cancelled.
//!
//! </div>
//!
//! ## Cancellation requests
//!
//! Any service with access to a [`ServiceContext`] can request the cancellation of _any other
//! service, at any point during execution_. This can be used for error handling for example, by
//! having a single service shut itself down without affecting other services, or for administrative
//! and testing purposes by having a node operator toggle services on and off from a remote
//! endpoint.
//!
//! You can use the following methods to request for the cancellation of a
//! service:
//!
//! - [`cancel_global`]: cancels all services.
//! - [`cancel_local`]: cancels this service and all its children.
//! - [`service_deactivate`]: cancel a specific service.
//!
//! ## Start requests
//!
//! You can _request_ for a service to be started by calling [`service_activate`]. Note that this will only
//! work if the service has already been registered at the start of the program.
//!
//! # Service orchestration
//!
//! Services are orchestrated by a [`ServiceMonitor`], which is responsible for registering services,
//! marking them as active or inactive as well as starting and restarting them upon request.
//! [`ServiceMonitor`] also handles the cancellation of all services upon receiving a `SIGINT` or
//! `SIGTERM`.
//!
//! <div class="warning">
//!
//! Services cannot be started or restarted if they have not been registered at startup.
//!
//! </div>
//!
//! ## example:
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::ServiceIdProvider;
//! # use mp_utils::service::ServiceRunner;
//! # use mp_utils::service::ServiceMonitorBuilder;
//! #
//! # pub struct MyService;
//! #
//! # #[async_trait::async_trait]
//! # impl Service for MyService {
//! #     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//! #         runner.service_loop(move |mut ctx| async move {
//! #             anyhow::Ok(())
//! #         })
//! #     }
//! # }
//! #
//! # impl ServiceIdProvider for MyService {
//! #     fn id_provider(&self) -> impl ServiceId {
//! #         MyServiceId
//! #     }
//! # }
//! #
//! # pub struct MyServiceId;
//! #
//! # impl ServiceId for MyServiceId {
//! #     fn svc_id(&self) -> String {
//! #         "MyService".to_string()
//! #     }
//! # }
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     ServiceMonitorBuilder::new()
//!         .await
//!         .with(MyService)?
//!         .activate(MyServiceId)
//!         .await?
//!         .start()
//!         .await
//! }
//! ```
//!
//! # Composing services
//!
//! Services can run other services, allowing you to create a [hierarchy] of services. Services
//! which manage other services are known as parent services, and services which are managed by
//! other services are known as [`child`] services. A service can be both a child and parent
//! service.If a parent service is cancelled, or stopped, then all of its child services will be
//! stopped as well.
//!
//! ## example:
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::ServiceIdProvider;
//! # use mp_utils::service::ServiceRunner;
//! # use mp_utils::service::ServiceMonitorBuilder;
//! struct ServiceA;
//! struct ServiceB;
//! struct ServiceC;
//!
//! enum MyServiceId {
//!     ServiceA,
//!     ServiceB,
//!     ServiceC,
//! }
//!
//! impl ServiceId for MyServiceId {
//!     fn svc_id(&self) -> String {
//!         match self {
//!             Self::ServiceA => "ServiceA".to_string(),
//!             Self::ServiceB => "ServiceB".to_string(),
//!             Self::ServiceC => "ServiceC".to_string(),
//!         }
//!     }
//! }
//!
//! #[async_trait::async_trait]
//! impl Service for ServiceA {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |ctx| async move {
//!             // Service A is the parent of services B and C. If A shuts down,
//!             // so will B and C.
//!             ctx.child()
//!                 .await
//!                 .with_active(ServiceB)
//!                 .await?
//!                 .with_active(ServiceC)
//!                 .await?
//!                 .start()
//!                 .await
//!         })
//!     }
//! }
//!
//! #[async_trait::async_trait]
//! impl Service for ServiceB {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |mut ctx| async move {
//!             ctx.cancelled().await;
//!             anyhow::Ok(())
//!         })
//!     }
//! }
//!
//! #[async_trait::async_trait]
//! impl Service for ServiceC {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |mut ctx| async move {
//!             ctx.cancelled().await;
//!             anyhow::Ok(())
//!         })
//!     }
//! }
//!
//! impl ServiceIdProvider for ServiceA {
//!     fn id_provider(&self) -> impl ServiceId {
//!         MyServiceId::ServiceA
//!     }
//! }
//!
//! impl ServiceIdProvider for ServiceB {
//!     fn id_provider(&self) -> impl ServiceId {
//!         MyServiceId::ServiceB
//!     }
//! }
//!
//! impl ServiceIdProvider for ServiceC {
//!     fn id_provider(&self) -> impl ServiceId {
//!         MyServiceId::ServiceC
//!     }
//! }
//! ```
//!
//! [`microservice`]: https://en.wikipedia.org/wiki/Microservices
//! [`service_loop`]: ServiceRunner::service_loop
//! [`cancelled`]: ServiceContext::run_until_cancelled
//! [`cancellation`]: ServiceContext::run_until_cancelled
//! [`is_cancelled`]: ServiceContext::is_cancelled
//! [`Poll::Pending`]: std::task::Poll::Pending
//! [`cancel_global`]: ServiceContext::cancel_global
//! [`cancel_local`]: ServiceContext::cancel_local
//! [`service_deactivate`]: ServiceContext::service_deactivate
//! [`service_activate`]: ServiceContext::service_activate
//! [hierarchy]: ServiceContext#scope
//! [`child`]: ServiceContext::child

use anyhow::Context;
use dashmap::DashMap;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::{
    collections::{btree_map, BTreeMap, BTreeSet},
    fmt::Debug,
    panic,
    sync::Arc,
    time::Duration,
};
use tokio::task::{JoinError, JoinSet};

use crate::{Freeze, Frozen};

/// Maximum duration a service is allowed to take to shutdown, after which it
/// will be forcefully cancelled
pub const SERVICE_GRACE_PERIOD: Duration = Duration::from_secs(10);

const CHAN_CAP: usize = 100;

/// An extensible type-safe wrapper around [`String`], used to identify a [`Service`][^1].
///
/// [^1]: See also: [`ServiceIdProvider`]
pub trait ServiceId {
    fn svc_id(&self) -> String;
}

/// Identifiers for all the [`Service`]s inside the Madara node
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MadaraServiceId {
    #[default]
    #[serde(skip)]
    Monitor,
    #[serde(skip)]
    Database,
    L1Sync,
    L2Sync,
    BlockProduction,
    #[serde(rename = "rpc")]
    RpcUser,
    #[serde(skip)]
    RpcAdmin,
    Gateway,
    Telemetry,
    P2P,
}

impl ServiceId for MadaraServiceId {
    fn svc_id(&self) -> String {
        match self {
            Self::Monitor => "monitor".to_string(),
            Self::Database => "database".to_string(),
            Self::L1Sync => "l1sync".to_string(),
            Self::L2Sync => "l2sync".to_string(),
            Self::BlockProduction => "blockprod".to_string(),
            Self::RpcUser => "rpcuser".to_string(),
            Self::RpcAdmin => "rpcadmin".to_string(),
            Self::Gateway => "gateway".to_string(),
            Self::Telemetry => "telemetry".to_string(),
            Self::P2P => "p2p".to_string(),
        }
    }
}

/// A [`Service`] can have three different statuses:
///
/// 1. [`Inactive`]: this is the default, the service has not been added to the [`ServiceMonitor`] or it
///    is no longer running.
/// 2. [`Active`]: a request was made to start a service. It is possible to request for services to be
///    started even if they are not registered with the [`ServiceMonitor`], in which case this will do
///    nothing.
/// 3. [`Running`]: the [`ServiceMonitor`] has started the service and it is running.
///
/// [`Inactive`]: Self::Inactive
/// [`Active`]: Self::Active
/// [`Running`]: Self::Running
#[derive(PartialEq, Eq, Clone, Copy, Default, Debug, Serialize, Deserialize)]
pub enum ServiceStatus {
    Active,
    Running,
    Shutdown,
    #[default]
    Inactive,
}

impl ServiceStatus {
    /// Check is a service is [`Running`](ServiceStatus)
    pub fn is_on(&self) -> bool {
        self == &ServiceStatus::Running
    }

    /// Checks if a service is [`Inactive`](ServiceStatus)
    pub fn is_off(&self) -> bool {
        self == &ServiceStatus::Inactive
    }
}

/// An atomic set used to store and interface with the state of [`Services`] which have been
/// registered with a [`ServiceMonitor`].
///
/// [`Services`]: Service
#[repr(transparent)]
#[derive(Default)]
struct ServiceSet(DashMap<String, ServiceStatus>);

impl ServiceSet {
    #[inline(always)]
    fn status(&self, id: &str) -> ServiceStatus {
        self.0.get(id).map(|cell| *cell.value()).unwrap_or_default()
    }

    #[inline(always)]
    fn set(&self, id: &str, status: ServiceStatus) -> ServiceStatus {
        self.0.insert(id.to_string(), status).unwrap_or_default()
    }

    #[inline(always)]
    fn unset(&self, id: &str) -> ServiceStatus {
        self.0.remove(id).map(|(_, v)| v).unwrap_or_default()
    }

    fn active_set(&self) -> &DashMap<String, ServiceStatus> {
        &self.0
    }
}

/// Provides info about updates to a [`Service`]'s status.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ServiceTransport {
    /// The [`ServiceId`] of the [`Service`] sending the update
    pub id_from: String,
    /// The [`ServiceId`] of the [`Service`] being updated.
    pub id_to: String,
    /// The new [`status`] of the [`Service`] being updated.
    ///
    /// [`status`]: ServiceStatus
    pub status: ServiceStatus,
}

/// Context information associated to a [`Service`], used for inter-service communication.
///
/// # Service hierarchy
///
/// - A services is said to be a _child service_ if it uses a context created with [`child`]
///
/// - A service is said to be a _parent service_ if it uses a context which was used to create child
///   services.
///
/// A parent services can always [`cancel`] all of its child services, but a child service cannot
/// cancel its parent service.
///
/// # Scope
///
/// You can create a hierarchy of services by calling [`child`]. Services are said to be in the same
/// _local scope_ if they are children of the same [`ServiceContext`], or children of those children
/// and so on. You can think of services being local if they can cancel each other without affecting
/// the rest of the app.
///
/// All services which are derived from the same [`ServiceContext`] are said to be in the same
/// _global scope_, that is to say any service in this scope can cancel _all_ other services in the
/// same scope (including child services) at any time. This is true of services in the same
/// [`ServiceMonitor`] for example.
///
/// Consider the following hierarchy of services:
///
/// ```text
///   A
///  / \
/// B   C
///    / \
///   D   E
/// ```
///
/// Service `A` can cancel all the services below it. Service `C` is a child of `A`, and a parent of
/// `D` and `E`. If `E` is [`cancelled`], it will also cancel services `D` and `E` but not `B`.
///
/// [`child`]: Self::child
/// [`cancel`]: Self::service_deactivate
/// [`cancelled`]: Self::cancel_local
pub struct ServiceContext {
    token_global: tokio_util::sync::CancellationToken,
    token_local: Option<tokio_util::sync::CancellationToken>,
    services: Arc<ServiceSet>,
    monitor: Arc<tokio::sync::mpsc::Sender<ServiceTransport>>,
    status_update: Arc<tokio::sync::Notify>,
    id: String,
}

impl Clone for ServiceContext {
    fn clone(&self) -> Self {
        Self {
            token_global: self.token_global.clone(),
            token_local: self.token_local.clone(),
            services: Arc::clone(&self.services),
            monitor: Arc::clone(&self.monitor),
            status_update: Arc::clone(&self.status_update),
            id: self.id.clone(),
        }
    }
}

impl ServiceContext {
    /// Creates a new [`Default`] [`ServiceContext`]
    fn new(sx: tokio::sync::mpsc::Sender<ServiceTransport>, id: impl ServiceId) -> Self {
        Self {
            token_global: tokio_util::sync::CancellationToken::new(),
            token_local: None,
            services: Arc::new(ServiceSet::default()),
            monitor: Arc::new(sx),
            status_update: Arc::default(),
            id: id.svc_id(),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn new_for_testing() -> Self {
        Self {
            token_global: tokio_util::sync::CancellationToken::new(),
            token_local: None,
            services: Arc::new(ServiceSet::default()),
            monitor: Arc::new(tokio::sync::mpsc::channel(0).0),
            status_update: Arc::default(),
            id: MadaraServiceId::Monitor.svc_id(),
        }
    }

    /// Stops all services under the same [global context scope].
    ///
    /// [local context scope]: Self#scope
    pub fn cancel_global(&self) {
        tracing::info!("🔌 Gracefully shutting down node");
        self.token_global.cancel();
        self.status_update.notify_waiters();
    }

    /// Stops all services under the same [local context scope].
    ///
    /// A local context is created by calling [`child`] and allows you to reduce the scope of
    /// cancellation only to those services which will use the new context.
    ///
    /// [local context scope]: Self#scope
    /// [`child`]: Self::child
    pub fn cancel_local(&self) {
        self.token_local.as_ref().unwrap_or(&self.token_global).cancel();
        self.status_update.notify_waiters();
    }

    pub async fn wait_cancel_global(&mut self) {
        self.cancel_global();
        self.wait_for_inactive(MadaraServiceId::Monitor).await;
    }

    pub async fn wait_cancel_local(&mut self) {
        let svc_id = self.id.clone();
        self.cancel_local();
        self.service_subscribe_for_impl(&svc_id, ServiceStatus::Inactive).await;
    }

    /// A future which completes when the [`Service`] associated to this [`ServiceContext`] is
    /// cancelled. Use this to race against other futures in a [`tokio::select`] or keep the
    /// [service loop] alive for as long as the service itself.
    ///
    /// This allows for more manual implementation of cancellation logic than [`run_until_cancelled`],
    /// and should only be used in cases where using `run_until_cancelled` is not possible or would
    /// be less clear.
    ///
    /// A service is cancelled after calling [`cancel_local`], [`cancel_global`] or if it is marked for
    /// removal with [`service_deactivate`].
    ///
    ///
    /// [`run_until_cancelled`]: Self::run_until_cancelled
    /// [`cancel_local`]: Self::cancel_local
    /// [`cancel_global`]: Self::cancel_global
    /// [`service_deactivate`]: Self::service_deactivate
    /// [service loop]: ServiceRunner::service_loop
    #[inline(always)]
    pub async fn cancelled(&mut self) {
        let token_global = &self.token_global;
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global);

        loop {
            // We keep checking for service status updates until a token has
            // been cancelled or this service was deactivated
            tokio::select! {
                _ = self.status_update.notified() => {},
                _ = token_global.cancelled() => {
                    break
                },
                _ = token_local.cancelled() => {
                    break
                }
            };

            let status = self.services.status(&self.id);
            if status == ServiceStatus::Shutdown || status == ServiceStatus::Inactive {
                return;
            }
        }
    }

    /// Checks if the [`Service`] associated to this [`ServiceContext`] was cancelled.
    ///
    /// A service is cancelled as a result of calling [`cancel_local`], [`cancel_global`] or
    /// [`service_deactivate`].
    ///
    /// # Limitations
    ///
    /// This function should _not_ be used when waiting on potentially blocking futures which can be
    /// cancelled without entering an invalid state. The latter is important, so let's break this
    /// down.
    ///
    /// - _blocking future_: this is blocking at a [`Service`] level, not at the node level. A
    ///   blocking task in this sense is a task which prevents a service from making progress in its
    ///   execution, but not necessarily the rest of the node. A prime example of this is when you
    ///   are waiting on a channel, and updates to that channel are sparse, or even unique.
    ///
    /// - _entering an invalid state_: the entire point of [`ServiceContext`] is to allow services to
    ///   gracefully shutdown. We do not want to be, for example, racing each service against a
    ///   global cancellation future, as not every service might be cancellation safe (we still do
    ///   this somewhat with [`SERVICE_GRACE_PERIOD`] but this is a last resort and should not execute
    ///   in normal circumstances).
    ///
    /// Put differently, we do not want to stop in the middle of a critical computation before it
    /// has been saved to disk, but we also do not want to leave the user hanging while we are
    /// waiting for some computation to complete.
    ///
    /// # When to use `is_cancelled`
    ///
    /// Putting this together, `is_cancelled` is only suitable for checking cancellation alongside
    /// tasks which:
    ///
    /// 1. Will not block the running service.
    /// 2. In very specific circumstances where we want the service to block the node if a
    ///    cancellation is requested.
    ///
    /// Examples of when to use `is_cancelled`:
    ///
    /// - All your computation does is sleep or tick away a (very) short period of time.
    /// - You are checking for cancellation inside of synchronous code.
    /// - You are performing a crucial task which should not be cancelled.
    ///
    /// Examples of when should _not_ use `is_cancelled`
    ///
    /// - You are waiting on a channel.
    /// - You are performing some long, expensive task.
    ///
    /// If you don't think you should be using `is_cancelled`, check out [`cancelled`] and
    /// [`run_until_cancelled`] instead.
    ///
    /// [`cancel_local`]: Self::cancel_local
    /// [`cancel_global`]: Self::cancel_global
    /// [`service_deactivate`]: Self::service_deactivate
    /// [`cancelled`]: Self::cancelled
    /// [`run_until_cancelled`]: Self::run_until_cancelled
    #[inline(always)]
    pub fn is_cancelled(&self) -> bool {
        let status = self.services.status(&self.id);
        self.token_global.is_cancelled()
            || self.token_local.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
            || status == ServiceStatus::Shutdown
            || status == ServiceStatus::Inactive
    }

    /// Runs a [`Future`] until the [`Service`] associated to this [`ServiceContext`] is cancelled.
    ///
    /// A service is cancelled as a result of calling [`cancel_local`], [`cancel_global`] or
    /// [`service_deactivate`].
    ///
    /// # Cancellation safety
    ///
    /// It is important that the future you pass to this function is _cancel- safe_ as it will be
    /// forcefully shutdown if ever the service is cancelled. This means your future might be
    /// interrupted at _any_ point in its execution.
    ///
    /// Futures can be considered as cancel-safe in the context of Madara if their computation can
    /// be interrupted at any point without causing any side-effects to the running node.
    ///
    /// # Returns
    ///
    /// The return value of the future wrapped in [`Some`], or [`None`] if the
    /// service was cancelled before the future could complete.
    ///
    /// [`cancel_local`]: Self::cancel_local
    /// [`cancel_global`]: Self::cancel_global
    /// [`service_deactivate`]: Self::service_deactivate
    pub async fn run_until_cancelled<T, F>(&mut self, f: F) -> Option<T>
    where
        T: Sized + Send + Sync,
        F: Future<Output = T>,
    {
        tokio::select! {
            res = f => Some(res),
            _ = self.cancelled() => None
        }
    }

    /// The id of the [`Service`] associated to this [`ServiceContext`]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Creates a new [`ServiceMonitorBuilder`] as a child of the current context.
    ///
    /// Any [`Service`] which is spawned from this new monitor will be able to cancel the services
    /// in the same [local scope] as itself, and any further child services, without affecting the
    /// rest of the [global scope].
    ///
    /// [local scope]: Self#scope
    /// [global scope]: Self#scope
    pub async fn child(&self) -> ServiceMonitorBuilder<ServiceMonitorBuilderStateNone> {
        let (sx, rx) = tokio::sync::mpsc::channel(CHAN_CAP);
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global).child_token();
        let ctx =
            Self { id: self.id.to_string(), token_local: Some(token_local), monitor: Arc::new(sx), ..self.clone() };

        ServiceMonitorBuilder::<ServiceMonitorBuilderStateNone>::new_with_ctx(rx, ctx).await
    }

    fn child_ctx(&self, id: &str) -> ServiceContext {
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global).child_token();
        Self { id: id.to_string(), token_local: Some(token_local), ..self.clone() }
    }

    /// Checks if a [`Service`] is running.
    #[inline(always)]
    pub fn service_status(&self, id: impl ServiceId) -> ServiceStatus {
        self.services.status(&id.svc_id())
    }

    /// Marks a [`Service`] as active.
    ///
    /// This will schedule for that service to be started if it is [`Inactive`] and it was registered
    /// via the [`ServiceMonitor`] at startup. This will immediately be visible to all services in the
    /// same global scope. This is true across threads.
    ///
    /// You can use [`service_subscribe`] to subscribe to changes in the status of any service.
    ///
    /// [`Inactive`]: ServiceStatus
    /// [global scope]: Self#scope
    /// [`service_subscribe`]: Self::service_subscribe
    #[inline(always)]
    pub async fn service_activate(&self, id: impl ServiceId) -> ServiceStatus {
        self.service_set(&id.svc_id(), ServiceStatus::Active).await
    }

    /// Marks a [`Service`] as inactive.
    ///
    /// This will schedule for that service to be shutdown if it is [`Running`] and it was registered
    /// via the [`ServiceMonitor`] at startup. This will immediately be visible to all services in the
    /// same [global scope]. This is true across threads.
    ///
    /// You can use [`service_subscribe`] to subscribe to changes in the status of any service.
    ///
    /// [`Running`]: ServiceStatus
    /// [global scope]: Self#scope
    /// [`service_subscribe`]: Self::service_subscribe
    #[inline(always)]
    pub async fn service_deactivate(&self, id: impl ServiceId) -> ServiceStatus {
        self.service_set(&id.svc_id(), ServiceStatus::Shutdown).await
    }

    #[inline(always)]
    pub async fn wait_activate(&self, id: impl ServiceId) -> ServiceStatus {
        let status = self.service_set(&id.svc_id(), ServiceStatus::Active).await;
        self.wait_for_running(id).await;
        status
    }

    #[inline(always)]
    pub async fn wait_deactivate(&self, id: impl ServiceId) -> ServiceStatus {
        let status = self.service_set(&id.svc_id(), ServiceStatus::Shutdown).await;
        self.wait_for_inactive(id).await;
        status
    }

    async fn service_set(&self, id: &str, status: ServiceStatus) -> ServiceStatus {
        let res = self.services.set(id, status);
        self.status_update.notify_waiters();
        if self
            .monitor
            .send(ServiceTransport { id_from: self.id.clone(), id_to: id.to_string(), status })
            .await
            .is_err()
        {
            self.cancel_global();
        };

        res
    }

    async fn service_unset(&self, id: &str) -> ServiceStatus {
        let res = self.services.unset(id);
        self.status_update.notify_waiters();
        if self
            .monitor
            .send(ServiceTransport { id_from: self.id.clone(), id_to: id.to_string(), status: ServiceStatus::Inactive })
            .await
            .is_err()
        {
            self.cancel_global();
        }

        res
    }

    /// Opens up a new subscription which will complete once _specific_ [`Service`] has reached a
    /// _specific_ [`status`].
    ///
    /// # Returns
    ///
    /// Identifying information about the service which was updated.
    ///
    /// [`status`]: ServiceStatus
    pub async fn service_subscribe_for(&self, id: impl ServiceId, status: ServiceStatus) -> ServiceStatus {
        self.service_subscribe_for_impl(&id.svc_id(), status).await
    }

    pub async fn service_subscribe_for_impl(&self, svc_id: &str, status: ServiceStatus) -> ServiceStatus {
        if self.services.status(svc_id) == status {
            return status;
        }

        loop {
            self.status_update.notified().await;
            if self.services.status(svc_id) == status {
                break status;
            }
        }
    }

    pub async fn wait_for_running(&self, id: impl ServiceId) -> ServiceStatus {
        self.service_subscribe_for(id, ServiceStatus::Running).await
    }

    pub async fn wait_for_inactive(&self, id: impl ServiceId) -> ServiceStatus {
        self.service_subscribe_for(id, ServiceStatus::Inactive).await
    }
}

/// A microservice in the Madara node.
///
/// The app is divided into services, with each service handling different responsibilities.
/// Depending on the startup configuration, some services are enabled and others disabled.
///
/// Services should be started with [`service_loop`].
///
/// # Writing your own service
///
/// Writing a service involves four steps:
///
/// 1. Implementing the [`ServiceId`] trait
/// 2. Implementing the [`Service`] trait
/// 3. Implementing the [`ServiceIdProvider`] trait
/// 4. Adding your service to a [`ServiceMonitor`] and running it.
///
/// ## example
///
/// ```rust
/// # use mp_utils::service::Service;
/// # use mp_utils::service::ServiceId;
/// # use mp_utils::service::ServiceIdProvider;
/// # use mp_utils::service::ServiceRunner;
/// # use mp_utils::service::ServiceMonitorBuilder;
/// // Step 1: implementing the `ServiceId` trait. We use this to identify our
/// // services.
/// pub enum MyServiceId {
///     MyServiceA,
///     MyServiceB
/// }
///
/// impl ServiceId for MyServiceId {
///     fn svc_id(&self) -> String {
///         match self {
///             Self::MyServiceA => "MyServiceA".to_string(),
///             Self::MyServiceB => "MyServiceB".to_string()
///         }
///     }
/// }
///
/// #[derive(Clone, Debug)]
/// pub enum Channel<T: Sized + Send + Sync> {
///     Open(T),
///     Closed
/// }
///
/// // Step 2: implementing the `Service` trait. An example service, sends over
/// // 4 integers to `ServiceB` and the exits
/// struct MyServiceA(tokio::sync::broadcast::Sender<Channel<usize>>);
///
/// #[async_trait::async_trait]
/// impl Service for MyServiceA {
///     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
///         let mut sx = self.0.clone();
///
///         runner.service_loop(move |mut ctx| async move {
///             for i in 0..4 {
///                 sx.send(Channel::Open(i))?;
///
///                 const SLEEP: std::time::Duration = std::time::Duration::from_millis(1);
///                 ctx.run_until_cancelled(tokio::time::sleep(SLEEP)).await;
///             }
///
///             // An important subtlety: we are using a broadcast channel to
///             // keep the connection alive between A and B even between
///             // restarts. To do this, we always keep a broadcast sender and
///             // receiver alive in A and B respectively, which we clone
///             // whenever either service starts. This means the channel won't
///             // close when the sender in A's service_loop is dropped! We need
///             // to explicitly notify B that it has received all the
///             // information A has to send to it, which is why we use the
///             //`Channel` enum.
///             sx.send(Channel::Closed);
///
///             anyhow::Ok(())
///
///         })
///     }
/// }
///
/// // Step 3: implementing the `ServiceIdProvider` trait. This re-uses the
/// // logic from step 1.
/// impl ServiceIdProvider for MyServiceA {
///     fn id_provider(&self) -> impl ServiceId {
///         MyServiceId::MyServiceA
///     }
/// }
///
/// // An example service, listens for messages from `ServiceA` and the exits
/// struct MyServiceB(tokio::sync::broadcast::Receiver<Channel<usize>>);
///
/// #[async_trait::async_trait]
/// impl Service for MyServiceB {
///     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
///         let mut rx = self.0.resubscribe();
///
///         runner.service_loop(move |mut ctx| async move {
///             loop {
///                 let i = tokio::select! {
///                     res = rx.recv() => {
///                         // As mentioned above, `res` will never receive an
///                         // `Err(RecvError::Closed)` since we always keep a
///                         // sender alive in A for restarts, so we manually
///                         // check if the channel was closed.
///                         match res? {
///                             Channel::Open(i) => i,
///                             Channel::Closed => break,
///                         }
///                     },
///                     // This is a case where using `ctx.run_until_cancelled`
///                     // would probably be harder to read.
///                     _ = ctx.cancelled() => break,
///                 };
///
///                 println!("MyServiceB received {i}");
///             }
///
///             anyhow::Ok(())
///         })
///     }
/// }
///
/// impl ServiceIdProvider for MyServiceB {
///     fn id_provider(&self) -> impl ServiceId {
///         MyServiceId::MyServiceB
///     }
/// }
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let (sx, rx) = tokio::sync::broadcast::channel(16);
///
///     let service_a = MyServiceA(sx);
///     let service_b = MyServiceB(rx);
///
///     // Step 4: we add our service to a `ServiceMonitor` (using a type-safe
///     // builder pattern)...
///     ServiceMonitorBuilder::new()
///         .await
///         .with_active(service_a)
///         .await?
///         .with_active(service_b)
///         .await?
///         .start() // ...and start them
///         .await
/// }
/// ```
///
/// [`service_loop`]: ServiceRunner::service_loop
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync + std::any::Any {
    /// Default impl does not start any task.
    #[allow(unused_variables)]
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Provides a way for a [`Service`] to identify itself in a type-safe way.
///
/// For reasons of dyn-compatibility, this is not part of the [`Service`] trait as we need to be able
/// to box services and this conflicts with `impl ServiceId`.
pub trait ServiceIdProvider {
    fn id_provider(&self) -> impl ServiceId;
}

#[async_trait::async_trait]
impl Service for Box<dyn Service> {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        self.as_mut().start(runner).await
    }
}

/// Entrypoint for [`Service`]s to start their main event loop.
///
/// Used to enforce certain shutdown behavior.
///
/// [`service_loop`]: Self::service_loop
pub struct ServiceRunner<'a> {
    ctx: ServiceContext,
    join_set: &'a mut JoinSet<anyhow::Result<String>>,
}

impl<'a> ServiceRunner<'a> {
    fn new(ctx: ServiceContext, join_set: &'a mut JoinSet<anyhow::Result<String>>) -> Self {
        Self { ctx, join_set }
    }

    /// The main loop of a [`Service`].
    ///
    /// The future passed to this function should complete _only once the service completes or is
    /// cancelled_. Services that complete early will automatically be cancelled.
    ///
    /// <div class="warning">
    ///
    /// As a safety mechanism, services have up to [`SERVICE_GRACE_PERIOD`] to gracefully shutdown
    /// before they are forcefully cancelled. This should not execute in a normal context and only
    /// serves to prevent infinite loops on shutdown request if services have not been implemented
    /// correctly
    ///
    /// </div>
    #[tracing::instrument(skip(self, runner), fields(module = "Service"))]
    pub fn service_loop<F, E>(self, runner: impl FnOnce(ServiceContext) -> F + Send + 'static) -> anyhow::Result<()>
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<anyhow::Error> + Send,
    {
        let Self { ctx, join_set } = self;
        join_set.spawn(async move {
            let id = ctx.id().to_string();
            tracing::debug!("Starting service with id: {id}");
            ctx.service_set(&id, ServiceStatus::Running).await;

            // If a service is implemented correctly, `stopper` should never
            // cancel first. This is a safety measure in case someone forgets to
            // implement a cancellation check along some branch of the service's
            // execution, or if they don't read the docs :D
            let ctx1 = ctx.clone();
            let ctx2 = ctx.clone();
            tokio::select! {
                res = runner(ctx1) => res.map_err(Into::into)?,
                _ = Self::stopper(ctx2, &id) => {},
            }

            tracing::debug!("Shutting down service with id: {id}");
            ctx.service_unset(&id).await;

            anyhow::Ok(id)
        });

        anyhow::Ok(())
    }

    async fn stopper(mut ctx: ServiceContext, id: &str) {
        ctx.cancelled().await;
        tokio::time::sleep(SERVICE_GRACE_PERIOD).await;

        tracing::warn!("⚠️  Forcefully shutting down service: {:?}", id);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceMonitorError {
    #[error("Service {0} has not been registered")]
    UnregisteredService(String),
    #[error("Service {0} has already been registered")]
    AlreadyRegisteredService(String),
}

pub struct ServiceMonitorBuilderStateNone;
pub struct ServiceMonitorBuilderStateSome;
pub struct ServiceMonitorBuilderStateSomeActive;

/// A type-safe builder around [`ServiceMonitor`].
///
/// [`ServiceMonitorBuilder`] is responsible for registering services and marking them as
/// [`Active`].
///
/// All services are [`Inactive`] by default. Only the services which are marked as _explicitly
/// active_ with [`activate`] will be automatically started when calling [`start`].
///
/// [`Active`]: ServiceStatus::Active
/// [`Inactive`]: ServiceStatus::Inactive
/// [`activate`]: Self::activate
/// [`start`]: Self::start
/// [`with`]: Self::with
pub struct ServiceMonitorBuilder<S> {
    services: BTreeMap<String, Box<dyn Service>>,
    status_monitored: BTreeSet<String>,
    monitored: BTreeSet<String>,
    ctx: ServiceContext,
    rx: tokio::sync::mpsc::Receiver<ServiceTransport>,
    _state: std::marker::PhantomData<S>,
}

impl ServiceMonitorBuilder<ServiceMonitorBuilderStateNone> {
    /// Creates a new [`ServiceMonitor`] builder.
    ///
    /// New services can be added and activated using [`with`], [`with_active`] and [`activate`].
    /// Call [`build`] or [`start`] once you are done. This requires you to have _added and
    /// activated_ at leat one service to the [`ServiceMonitorBuilder`].
    ///
    /// [`with`]: Self::with
    /// [`with_active`]: Self::with_active
    /// [`activate`]: Self::activate
    /// [`build`]: Self::build
    /// [`start`]: Self::start
    pub async fn new() -> ServiceMonitorBuilder<ServiceMonitorBuilderStateNone> {
        let (sx, rx) = tokio::sync::mpsc::channel(CHAN_CAP);
        let ctx = ServiceContext::new(sx, MadaraServiceId::Monitor);
        ServiceMonitorBuilder::<ServiceMonitorBuilderStateNone>::new_with_ctx(rx, ctx).await
    }

    /// Registers a [`Service`] to the [`ServiceMonitor`]. This service is [`Inactive`] by default and
    /// needs to be marked as [`Active`] by calling [`activate`]. Only active services will be started
    /// when calling [`start`].
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyRegisteredService`] if the service had already been added to the
    /// [`ServiceMonitorBuilder`].
    ///
    /// [`Inactive`]: ServiceStatus
    /// [`Active`]: ServiceStatus
    /// [`activate`]: Self::activate
    /// [`start`]: Self::start
    /// [`AlreadyRegisteredService`]: ServiceMonitorError::AlreadyRegisteredService
    pub fn with(
        self,
        svc: impl Service + ServiceIdProvider,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSome>, ServiceMonitorError> {
        self.with_impl(svc)
    }
}

impl ServiceMonitorBuilder<ServiceMonitorBuilderStateSome> {
    /// Registers a [`Service`] to the [`ServiceMonitor`]. This service is [`Inactive`] by default and
    /// needs to be marked as [`Active`] by calling [`activate`]. Only active services will be started
    /// when calling [`start`].
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyRegisteredService`] if the service had already been added to the
    /// [`ServiceMonitorBuilder`].
    ///
    /// [`Inactive`]: ServiceStatus
    /// [`Active`]: ServiceStatus
    /// [`activate`]: Self::activate
    /// [`start`]: Self::start
    /// [`AlreadyRegisteredService`]: ServiceMonitorError::AlreadyRegisteredService
    pub fn with(
        self,
        svc: impl Service + ServiceIdProvider,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSome>, ServiceMonitorError> {
        self.with_impl(svc)
    }

    /// Marks a [`Service`] as [`Active`], meaning it will be started automatically when calling
    /// [`start`].
    ///
    /// # Errors
    ///
    /// Returns [`UnregisteredService`] if trying to activate a service that has not already been
    /// added by using [`with`].
    ///
    /// [`Active`]: ServiceStatus
    /// [`start`]: Self::start
    /// [`UnregisteredService`]: ServiceMonitorError::UnregisteredService
    /// [`with`]: Self::with
    pub async fn activate(
        self,
        id: impl ServiceId,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive>, ServiceMonitorError> {
        self.activate_impl(&id.svc_id()).await
    }

    /// Marks a [`Service`] as [`Active`], if a condition is met, in which case it will be started
    /// automatically when calling [`start`].
    ///
    /// Note that this is not entirely type safe as we cannot encode runtime assertions into the
    /// type system.
    ///
    /// # Errors
    ///
    /// Returns [`UnregisteredService`] if trying to activate a service that has not already been
    /// added by using [`with`].
    ///
    /// [`Active`]: ServiceStatus
    /// [`start`]: Self::start
    /// [`UnregisteredService`]: ServiceMonitorError::UnregisteredService
    /// [`with`]: Self::with
    pub async fn activate_if(
        self,
        id: impl ServiceId,
        f: impl Fn() -> bool,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive>, ServiceMonitorError> {
        self.activate_if_impl(id, f).await
    }
}

impl ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive> {
    /// Registers a [`Service`] to the [`ServiceMonitor`]. This service is [`Inactive`] by default and
    /// needs to be marked as [`Active`] by calling [`activate`]. Only active services will be started
    /// when calling [`start`].
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyRegisteredService`] if the service had already been added to the
    /// [`ServiceMonitorBuilder`].
    ///
    /// [`Inactive`]: ServiceStatus
    /// [`Active`]: ServiceStatus
    /// [`activate`]: Self::activate
    /// [`start`]: Self::start
    /// [`AlreadyRegisteredService`]: ServiceMonitorError::AlreadyRegisteredService
    pub fn with(
        self,
        svc: impl Service + ServiceIdProvider,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive>, ServiceMonitorError> {
        self.with_impl(svc)
    }

    /// Marks a [`Service`] as [`Active`], meaning it will be started automatically when calling
    /// [`start`].
    ///
    /// # Errors
    ///
    /// Returns [`UnregisteredService`] if trying to activate a service that has not already been
    /// added by using [`with`].
    ///
    /// [`Active`]: ServiceStatus
    /// [`start`]: Self::start
    /// [`UnregisteredService`]: ServiceMonitorError::UnregisteredService
    /// [`with`]: Self::with
    pub async fn activate(
        self,
        id: impl ServiceId,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive>, ServiceMonitorError> {
        self.activate_impl(&id.svc_id()).await
    }

    /// Marks a [`Service`] as [`Active`], if a condition is met, in which case it will be started
    /// automatically when calling [`start`].
    ///
    /// Note that this is not entirely type safe as we cannot encode runtime assertions into the
    /// type system.
    ///
    /// # Errors
    ///
    /// Returns [`UnregisteredService`] if trying to activate a service that has not already been
    /// added by using [`with`].
    ///
    /// [`Active`]: ServiceStatus
    /// [`start`]: Self::start
    /// [`UnregisteredService`]: ServiceMonitorError::UnregisteredService
    /// [`with`]: Self::with
    pub async fn activate_if(
        self,
        id: impl ServiceId,
        f: impl Fn() -> bool,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive>, ServiceMonitorError> {
        self.activate_if_impl(id, f).await
    }

    /// Consumes this builder and returns a [ServiceMonitor]
    pub fn build(self) -> ServiceMonitor {
        let Self { services, status_monitored, monitored, ctx, rx, .. } = self;
        ServiceMonitor {
            services,
            join_set: Default::default(),
            status_actual: Default::default(),
            status_monitored,
            monitored: monitored.freeze(),
            ctx,
            rx,
        }
    }

    /// Starts all activate [`Service`]s and runs them to completion. Services are activated by
    /// calling [`activate`]. This function completes once all services have been run to completion.
    ///
    /// <div class="warning">
    ///
    /// Keep in mind that services can only be restarted as long as other services are running
    /// (otherwise the node would shutdown).
    ///
    /// </div>
    ///
    /// [`activate`]: Self::activate
    pub async fn start(self) -> anyhow::Result<()> {
        self.build().start().await
    }
}

impl<S> ServiceMonitorBuilder<S> {
    /// Registers a [`Service`] to the [`ServiceMonitor`] and marks it as [`Active`].
    ///
    /// See also: [`with`], [`activate`]
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyRegisteredService`] if the service had already been added to the
    /// [`ServiceMonitorBuilder`].
    ///
    /// [`Active`]: ServiceStatus
    /// [`with`]: Self::with
    /// [`activate`]: Self::activate
    /// [`AlreadyRegisteredService`]: ServiceMonitorError::AlreadyRegisteredService
    pub async fn with_active(
        mut self,
        svc: impl Service + ServiceIdProvider,
    ) -> Result<ServiceMonitorBuilder<ServiceMonitorBuilderStateSomeActive>, ServiceMonitorError> {
        let svc_id = svc.id_provider().svc_id();
        self = self.with_impl(svc)?;
        self.activate_impl(&svc_id).await
    }

    async fn new_with_ctx(
        rx: tokio::sync::mpsc::Receiver<ServiceTransport>,
        ctx: ServiceContext,
    ) -> ServiceMonitorBuilder<ServiceMonitorBuilderStateNone> {
        let id = ctx.id.clone();
        ctx.service_set(&id, ServiceStatus::Active).await;
        ServiceMonitorBuilder {
            services: Default::default(),
            status_monitored: Default::default(),
            monitored: Default::default(),
            ctx,
            rx,
            _state: std::marker::PhantomData,
        }
    }

    pub fn with_impl<S2>(
        self,
        svc: impl Service + ServiceIdProvider,
    ) -> Result<ServiceMonitorBuilder<S2>, ServiceMonitorError> {
        let Self { mut services, status_monitored, mut monitored, ctx, rx, .. } = self;

        let svc_id = svc.id_provider().svc_id();
        match services.entry(svc_id.clone()) {
            btree_map::Entry::Vacant(entry) => {
                entry.insert(Box::new(svc));
                monitored.insert(svc_id);

                Ok(ServiceMonitorBuilder {
                    services,
                    status_monitored,
                    monitored,
                    ctx,
                    rx,
                    _state: std::marker::PhantomData,
                })
            }
            btree_map::Entry::Occupied(_) => Err(ServiceMonitorError::AlreadyRegisteredService(svc_id)),
        }
    }

    async fn activate_impl<S2>(mut self, svc_id: &str) -> Result<ServiceMonitorBuilder<S2>, ServiceMonitorError> {
        if !self.services.contains_key(svc_id) {
            Err(ServiceMonitorError::UnregisteredService(svc_id.to_string()))
        } else {
            self.ctx.service_set(svc_id, ServiceStatus::Active).await;
            self.status_monitored.insert(svc_id.to_string());

            let Self { services, status_monitored, monitored, ctx, rx, .. } = self;
            Ok(ServiceMonitorBuilder {
                services,
                status_monitored,
                monitored,
                ctx,
                rx,
                _state: std::marker::PhantomData,
            })
        }
    }

    async fn activate_if_impl<S2>(
        self,
        id: impl ServiceId,
        f: impl Fn() -> bool,
    ) -> Result<ServiceMonitorBuilder<S2>, ServiceMonitorError> {
        if f() {
            self.activate_impl(&id.svc_id()).await
        } else {
            let Self { services, status_monitored, monitored, ctx, rx, .. } = self;
            Ok(ServiceMonitorBuilder {
                services,
                status_monitored,
                monitored,
                ctx,
                rx,
                _state: std::marker::PhantomData,
            })
        }
    }
}

/// Orchestrates the execution of various [`Service`]s.
///
/// A [`ServiceMonitor`] is responsible for starting and stopping services as well as handling
/// `SIGINT` and `SIGTERM`. Services are run to completion until no service remains, at which point
/// the node will automatically shutdown.
///
/// Note that services which are not present at startup cannot be started or restarted.
pub struct ServiceMonitor {
    services: BTreeMap<String, Box<dyn Service>>, // <
    join_set: JoinSet<anyhow::Result<String>>,
    status_actual: ServiceSet,
    status_monitored: BTreeSet<String>,
    monitored: Frozen<BTreeSet<String>>,
    ctx: ServiceContext,
    rx: tokio::sync::mpsc::Receiver<ServiceTransport>,
}

impl ServiceMonitor {
    /// Starts all activate [`Service`]s and runs them to completion. Services are activated by
    /// calling [`activate`]. This function completes once all services have been run to completion.
    ///
    /// <div class="warning">
    ///
    /// Keep in mind that services can only be restarted as long as other services are running
    /// (otherwise the node would shutdown).
    ///
    /// </div>
    ///
    /// [`activate`]: ServiceMonitorBuilder::activate
    #[tracing::instrument(skip(self), fields(module = "Service"))]
    pub async fn start(mut self) -> anyhow::Result<()> {
        self.register_services().await?;
        self.register_close_handles().await?;

        tracing::debug!("Running services: {:?}", self.ctx.services.active_set());

        while !self.status_monitored.is_empty() {
            tokio::select! {
                // A service has run to completion, mark it as inactive
                Some(result) = self.join_set.join_next() => self.service_deactivate(result)?,
                // A service has had its status updated, check if it is a start request
                Some(transport) = self.rx.recv() => self.service_activate(transport).await?,
                // The service running this monitor has been cancelled and we should exit here
                _ = self.ctx.cancelled() => self.service_shutdown().await?,
                else => continue
            };

            tracing::debug!("Services still active: {:?}", self.ctx.services.active_set());
        }

        if self.ctx.id == MadaraServiceId::Monitor.svc_id() {
            let id = self.ctx.id.clone();
            self.ctx.service_unset(&id).await;
        }

        Ok(())
    }

    async fn register_services(&mut self) -> anyhow::Result<()> {
        if self.status_monitored.is_empty() {
            // TODO: move this to a concrete error type
            tracing::error!("No active services at startup, shutting down monitor...");
            anyhow::bail!("No active services at startup");
        }

        // start only the initially active services
        let id = self.ctx.id.clone();
        self.ctx.service_set(&id, ServiceStatus::Running).await;
        for (id, svc) in self.services.iter_mut() {
            if self.ctx.services.status(id) == ServiceStatus::Active {
                self.status_actual.set(id, ServiceStatus::Active);

                let ctx = self.ctx.child_ctx(id.as_str());
                let runner = ServiceRunner::new(ctx, &mut self.join_set);
                svc.start(runner).await.context("Starting service")?;

                self.status_actual.set(id, ServiceStatus::Running);
                self.ctx.service_subscribe_for_impl(id, ServiceStatus::Running).await;
            }
        }

        anyhow::Ok(())
    }

    async fn register_close_handles(&mut self) -> anyhow::Result<()> {
        let runner = ServiceRunner::new(self.ctx.clone(), &mut self.join_set);

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
        })
    }

    fn service_deactivate(&mut self, svc_res: Result<anyhow::Result<String>, JoinError>) -> anyhow::Result<()> {
        match svc_res {
            Ok(result) => {
                let id = result?;
                tracing::debug!("Service {id:?} has shut down");

                // TODO: add invariant checks here
                self.status_actual.unset(&id);
                self.status_monitored.remove(&id);
            }
            Err(panic_error) if panic_error.is_panic() => {
                // bubble up panics too
                panic::resume_unwind(panic_error.into_panic());
            }
            Err(_task_cancelled_error) => {}
        };

        anyhow::Ok(())
    }

    async fn service_shutdown(&mut self) -> anyhow::Result<()> {
        tracing::debug!("Service monitor {} is being shut down", self.ctx.id());
        for id in self.monitored.iter() {
            tracing::debug!("Waiting for {id} to be inactive");
            self.ctx.clone().service_subscribe_for_impl(id.as_str(), ServiceStatus::Inactive).await;
            self.status_monitored.remove(id.as_str());
            tracing::debug!("{id} is inactive");
        }

        anyhow::Ok(())
    }

    async fn service_activate(&mut self, transport: ServiceTransport) -> anyhow::Result<()> {
        let ServiceTransport { id_to, status, .. } = transport;
        if status == ServiceStatus::Active {
            if let Some(svc) = self.services.get_mut(&id_to) {
                if self.status_actual.status(&id_to) == ServiceStatus::Inactive {
                    let is_monitored = self.monitored.contains(&id_to);

                    self.ctx.service_set(&id_to, ServiceStatus::Active).await;
                    self.status_actual.set(&id_to, ServiceStatus::Active);
                    if is_monitored {
                        self.status_monitored.insert(id_to.clone());
                    }

                    let ctx = self.ctx.child_ctx(&id_to);
                    let runner = ServiceRunner::new(ctx, &mut self.join_set);
                    let res = svc.start(runner).await;

                    if res.is_err() {
                        tracing::error!("Service {id_to} failed to start");
                        self.status_actual.unset(&id_to);
                        self.status_monitored.remove(&id_to);
                        self.ctx.service_set(&id_to, ServiceStatus::Shutdown).await;
                        return res.context("Starting service");
                    }

                    self.status_actual.set(&id_to, ServiceStatus::Running);
                    if is_monitored {
                        self.status_monitored.insert(id_to.clone());
                    }

                    tracing::debug!("Service {id_to} has started");
                }
            }
        };

        anyhow::Ok(())
    }

    pub fn ctx(&self) -> ServiceContext {
        self.ctx.clone()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const TIMEOUT: u64 = 100;

    enum ServiceIdTest {
        ServiceA,
        ServiceB,
        ServiceC,
        ServiceD,
        ServiceE,
    }

    impl ServiceId for ServiceIdTest {
        fn svc_id(&self) -> String {
            match self {
                ServiceIdTest::ServiceA => "ServiceA".to_string(),
                ServiceIdTest::ServiceB => "ServiceB".to_string(),
                ServiceIdTest::ServiceC => "ServiceC".to_string(),
                ServiceIdTest::ServiceD => "ServiceD".to_string(),
                ServiceIdTest::ServiceE => "ServiceE".to_string(),
            }
        }
    }

    async fn service_waiting(runner: ServiceRunner<'_>) -> anyhow::Result<()> {
        runner.service_loop(move |mut cx| async move {
            cx.cancelled().await;
            anyhow::Ok(())
        })
    }

    struct ServiceAWaiting;
    struct ServiceBWaiting;

    #[async_trait::async_trait]
    impl Service for ServiceAWaiting {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            service_waiting(runner).await
        }
    }

    #[async_trait::async_trait]
    impl Service for ServiceBWaiting {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            service_waiting(runner).await
        }
    }

    impl ServiceIdProvider for ServiceAWaiting {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceA
        }
    }

    impl ServiceIdProvider for ServiceBWaiting {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceB
        }
    }

    #[derive(Clone, Default)]
    struct ServiceAParent {
        a: Arc<tokio::sync::Notify>,
        b: Arc<tokio::sync::Notify>,
        c: Arc<tokio::sync::Notify>,
        d: Arc<tokio::sync::Notify>,
        e: Arc<tokio::sync::Notify>,
    }
    struct ServiceBChild {
        b: Arc<tokio::sync::Notify>,
    }
    struct ServiceCParent {
        c: Arc<tokio::sync::Notify>,
        d: Arc<tokio::sync::Notify>,
        e: Arc<tokio::sync::Notify>,
    }
    struct ServiceDChild {
        d: Arc<tokio::sync::Notify>,
    }
    struct ServiceEChild {
        e: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl Service for ServiceAParent {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            let a = self.a.clone();
            let b = self.b.clone();
            let c = self.c.clone();
            let d = self.d.clone();
            let e = self.e.clone();

            runner.service_loop(move |mut cx| async move {
                let cx1 = cx.clone();
                tokio::join!(
                    cx.child()
                        .await
                        .with_active(ServiceBChild { b })
                        .await
                        .expect("Failed to add service B")
                        .with_active(ServiceCParent { c, d, e })
                        .await
                        .expect("Failed to add service C")
                        .start(),
                    cx.run_until_cancelled(async {
                        a.notified().await;
                        cx1.cancel_local();
                    })
                )
                .0?;

                println!("Exited A!");

                cx.cancelled().await;
                anyhow::Ok(())
            })
        }
    }

    #[async_trait::async_trait]
    impl Service for ServiceBChild {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            let b = self.b.clone();
            runner.service_loop(move |cx| async move {
                let mut cx1 = cx;
                let mut cx2 = cx1.clone();
                let cx3 = cx1.clone();

                tokio::join!(
                    cx1.cancelled(),
                    cx2.run_until_cancelled(async {
                        b.notified().await;
                        cx3.cancel_local()
                    })
                );

                println!("Received cancellation inside of service B");

                anyhow::Ok(())
            })
        }
    }

    #[async_trait::async_trait]
    impl Service for ServiceCParent {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            let c = self.c.clone();
            let d = self.d.clone();
            let e = self.e.clone();

            runner.service_loop(move |mut cx| async move {
                let cx1 = cx.clone();
                tokio::join!(
                    cx.child()
                        .await
                        .with_active(ServiceDChild { d })
                        .await
                        .expect("Failed to add service D")
                        .with_active(ServiceEChild { e })
                        .await
                        .expect("Failed to add service E")
                        .start(),
                    cx.run_until_cancelled(async {
                        c.notified().await;
                        cx1.cancel_local();
                    })
                )
                .0
            })
        }
    }

    #[async_trait::async_trait]
    impl Service for ServiceDChild {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            let d = self.d.clone();
            runner.service_loop(move |cx| async move {
                let mut cx1 = cx;
                let mut cx2 = cx1.clone();
                let cx3 = cx1.clone();

                tokio::join!(
                    cx1.cancelled(),
                    cx2.run_until_cancelled(async {
                        d.notified().await;
                        cx3.cancel_local()
                    })
                );

                println!("Received cancellation inside of service D");

                anyhow::Ok(())
            })
        }
    }

    #[async_trait::async_trait]
    impl Service for ServiceEChild {
        async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
            let e = self.e.clone();
            runner.service_loop(move |cx| async move {
                let mut cx1 = cx;
                let mut cx2 = cx1.clone();
                let cx3 = cx1.clone();

                tokio::join!(
                    cx1.cancelled(),
                    cx2.run_until_cancelled(async {
                        e.notified().await;
                        cx3.cancel_local()
                    })
                );

                println!("Received cancellation inside of service E");

                anyhow::Ok(())
            })
        }
    }

    impl ServiceIdProvider for ServiceAParent {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceA
        }
    }

    impl ServiceIdProvider for ServiceBChild {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceB
        }
    }

    impl ServiceIdProvider for ServiceCParent {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceC
        }
    }

    impl ServiceIdProvider for ServiceDChild {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceD
        }
    }

    impl ServiceIdProvider for ServiceEChild {
        fn id_provider(&self) -> impl ServiceId {
            ServiceIdTest::ServiceE
        }
    }

    async fn with_monitor<F>(monitor: ServiceMonitor, f: impl FnOnce(ServiceContext) -> F) -> ServiceContext
    where
        F: Future<Output = ()>,
    {
        let ctx = monitor.ctx.clone();
        tokio::join!(
            async {
                monitor.start().await.expect("Failed to start monitor");
            },
            f(ctx.clone())
        );

        ctx
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(10 * TIMEOUT))]
    async fn service_context_wait_for() {
        let monitor = ServiceMonitorBuilder::new()
            .await
            .with(ServiceAWaiting)
            .expect("Failed to start service A")
            .with_active(ServiceBWaiting)
            .await
            .expect("Failed to start service B")
            .build();

        let ctx = with_monitor(monitor, |ctx| async move {
            let ctx1 = ctx.clone();

            tokio::join!(
                async {
                    assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Inactive);
                    ctx.wait_for_running(ServiceIdTest::ServiceA).await;
                    assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Running);

                    ctx.service_deactivate(ServiceIdTest::ServiceA).await;
                    ctx.wait_for_inactive(ServiceIdTest::ServiceA).await;
                    assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Inactive);

                    ctx.service_deactivate(ServiceIdTest::ServiceB).await;
                    ctx.wait_for_inactive(ServiceIdTest::ServiceB).await;
                    assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
                },
                async {
                    ctx1.wait_for_running(ServiceIdTest::ServiceB).await;
                    assert_eq!(ctx1.service_status(ServiceIdTest::ServiceB), ServiceStatus::Running);

                    tokio::time::sleep(std::time::Duration::from_millis(TIMEOUT)).await;
                    ctx1.service_activate(ServiceIdTest::ServiceA).await;
                }
            );
        })
        .await;

        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Inactive);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(10 * TIMEOUT))]
    async fn service_context_cancel_global() {
        let monitor = ServiceMonitorBuilder::new()
            .await
            .with_active(ServiceAParent::default())
            .await
            .expect("Failed to add Service A")
            .build();

        let ctx = monitor.ctx().clone();
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Active);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Active);

        with_monitor(monitor, |ctx| async move {
            ctx.wait_for_running(ServiceIdTest::ServiceA).await;
            ctx.wait_for_running(ServiceIdTest::ServiceB).await;
            ctx.wait_for_running(ServiceIdTest::ServiceC).await;
            ctx.wait_for_running(ServiceIdTest::ServiceD).await;
            ctx.wait_for_running(ServiceIdTest::ServiceE).await;

            ctx.cancel_global();

            ctx.wait_for_inactive(ServiceIdTest::ServiceA).await;
            ctx.wait_for_inactive(ServiceIdTest::ServiceB).await;
            ctx.wait_for_inactive(ServiceIdTest::ServiceC).await;
            ctx.wait_for_inactive(ServiceIdTest::ServiceD).await;
            ctx.wait_for_inactive(ServiceIdTest::ServiceE).await;
            ctx.wait_for_inactive(MadaraServiceId::Monitor).await;
        })
        .await;

        assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Inactive);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(1000))]
    async fn service_monitor_simple() {
        let monitor = ServiceMonitorBuilder::new()
            .await
            .with_active(ServiceAWaiting)
            .await
            .expect("Failed to add Service A")
            .with_active(ServiceBWaiting)
            .await
            .expect("Failed to add Service B")
            .build();

        let ctx = monitor.ctx();
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Active);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Active);
        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Active);

        with_monitor(monitor, |ctx| async move {
            assert_eq!(
                ctx.service_subscribe_for(ServiceIdTest::ServiceA, ServiceStatus::Running).await,
                ServiceStatus::Running
            );
            assert_eq!(
                ctx.service_subscribe_for(ServiceIdTest::ServiceB, ServiceStatus::Running).await,
                ServiceStatus::Running
            );

            ctx.service_deactivate(ServiceIdTest::ServiceA).await;
            assert_eq!(
                ctx.service_subscribe_for(ServiceIdTest::ServiceA, ServiceStatus::Inactive).await,
                ServiceStatus::Inactive
            );

            ctx.service_deactivate(ServiceIdTest::ServiceB).await;
            assert_eq!(
                ctx.service_subscribe_for(ServiceIdTest::ServiceB, ServiceStatus::Inactive).await,
                ServiceStatus::Inactive
            );
        })
        .await;

        assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Inactive);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(TIMEOUT))]
    async fn service_context_cancellation_scope_1() {
        let service_a = ServiceAParent::default();
        let monitor = ServiceMonitorBuilder::new()
            .await
            .with_active(service_a.clone())
            .await
            .expect("Failed to add Service A")
            .build();

        let ctx = with_monitor(monitor, |ctx| async move {
            ctx.wait_for_running(ServiceIdTest::ServiceA).await;
            ctx.wait_for_running(ServiceIdTest::ServiceB).await;
            ctx.wait_for_running(ServiceIdTest::ServiceC).await;
            ctx.wait_for_running(ServiceIdTest::ServiceD).await;
            ctx.wait_for_running(ServiceIdTest::ServiceE).await;

            service_a.e.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceE).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);

            service_a.d.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceD).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);

            service_a.c.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceC).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);

            service_a.b.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceB).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);

            service_a.a.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceA).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);
        })
        .await;

        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Inactive);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(TIMEOUT))]
    async fn service_context_cancellation_scope_2() {
        let service_a = ServiceAParent::default();
        let monitor = ServiceMonitorBuilder::new()
            .await
            .with_active(service_a.clone())
            .await
            .expect("Failed to add Service A")
            .build();

        let ctx = with_monitor(monitor, |ctx| async move {
            ctx.wait_for_running(ServiceIdTest::ServiceA).await;
            ctx.wait_for_running(ServiceIdTest::ServiceB).await;
            ctx.wait_for_running(ServiceIdTest::ServiceC).await;
            ctx.wait_for_running(ServiceIdTest::ServiceD).await;
            ctx.wait_for_running(ServiceIdTest::ServiceE).await;

            service_a.c.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceC).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Running);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);

            service_a.a.notify_waiters();
            ctx.wait_for_inactive(ServiceIdTest::ServiceA).await;
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceA), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceB), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceC), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceD), ServiceStatus::Inactive);
            assert_eq!(ctx.service_status(ServiceIdTest::ServiceE), ServiceStatus::Inactive);
        })
        .await;

        assert_eq!(ctx.service_status(MadaraServiceId::Monitor), ServiceStatus::Inactive);
    }
}
