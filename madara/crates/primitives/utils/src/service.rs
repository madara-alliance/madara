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
//! identified by a single [std::sync::atomic::AtomicU64]. More about this later.
//!
//! Services are started with [Service::start] using [ServiceRunner::service_loop].
//! [ServiceRunner::service_loop] is a function which takes in a future: this
//! future represents the main loop of your service, and should run until your
//! service completes or is cancelled.
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
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::PowerOfTwo;
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
//! }
//!
//! impl ServiceId for MyService {
//!     fn svc_id(&self) -> PowerOfTwo {
//!         MadaraServiceId::Monitor.svc_id()
//!     }
//! }
//! ```
//!
//! ## A correct implementation of the [Service] trait
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::PowerOfTwo;
//! # use mp_utils::service::ServiceRunner;
//! # use mp_utils::service::MadaraServiceId;
//!
//! pub struct MyService;
//!
//! #[async_trait::async_trait]
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |mut ctx| async move {
//!             ctx.run_until_cancelled(tokio::time::sleep(std::time::Duration::MAX)).await;
//!
//!             // This is correct, as the future passed to service_loop will
//!             // only resolve once the task above completes, so Madara can
//!             // correctly mark this service as ready to restart.
//!             anyhow::Ok(())
//!         });
//!
//!         anyhow::Ok(())
//!     }
//! }
//!
//! impl ServiceId for MyService {
//!     fn svc_id(&self) -> PowerOfTwo {
//!         MadaraServiceId::Monitor.svc_id()
//!     }
//! }
//! ```
//!
//! Or if you really need to spawn a background task:
//!
//! ```rust
//! # use mp_utils::service::Service;
//! # use mp_utils::service::ServiceId;
//! # use mp_utils::service::PowerOfTwo;
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
//!                 ctx1.run_until_cancelled(tokio::time::sleep(std::time::Duration::MAX)).await;
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
//! }
//!
//! impl ServiceId for MyService {
//!     fn svc_id(&self) -> PowerOfTwo {
//!         MadaraServiceId::Monitor.svc_id()
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
//! Note that service shutdown is designed to be manual. We still implement a
//! [SERVICE_GRACE_PERIOD] which is the maximum duration a service is allowed
//! to take to shutdown, after which it is forcefully cancelled. This should not
//! happen in practice and only serves to avoid cases where someone would forget
//! to implement a cancellation check. More on this in the next section.
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
//! > cancelled.
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
//! [std::sync::atomic::AtomicU64] bitmask with strong [std::sync::atomic::Ordering::SeqCst]
//! cross-thread ordering of operations. Services are represented as a unique
//! [PowerOfTwo] which is provided through the [ServiceId] trait.
//!
//! > **Note**
//! > The use of [std::sync::atomic::AtomicU64] limits the number of possible
//! > services to 64. This might be increased in the future if there is a
//! > demonstrable need for more services, but right now this limit seems
//! > high enough.
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

/// Maximum potential number of services that a [ServiceRunner] can run at once
pub const SERVICE_COUNT_MAX: usize = 64;

/// Maximum duration a service is allowed to take to shutdown, after which it
/// will be forcefully cancelled
pub const SERVICE_GRACE_PERIOD: Duration = Duration::from_secs(10);

macro_rules! power_of_two {
    ( $($pow:literal),* ) => {
        paste::paste! {
            #[repr(u64)]
            #[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
            pub enum PowerOfTwo {
                #[default]
                ZERO = 0,
                $(
                    [<P $pow>] = 1u64 << $pow,
                )*
            }

            impl PowerOfTwo {
                /// Converts a [PowerOfTwo] into a unique index which can be
                /// used in an arrray
                pub fn index(&self) -> usize {
                    match self {
                        Self::ZERO => 0,
                        $(
                            Self::[<P $pow>] => $pow,
                        )*
                    }
                }
            }

            impl ServiceId for PowerOfTwo {
                fn svc_id(&self) -> PowerOfTwo {
                    *self
                }
            }

            impl Display for PowerOfTwo {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{}", *self as u64)
                }
            }

            impl TryFrom<u8> for PowerOfTwo {
                type Error = anyhow::Error;

                fn try_from(pow: u8) -> anyhow::Result<Self> {
                    TryFrom::<u64>::try_from(pow as u64)
                }
            }

            impl TryFrom<u16> for PowerOfTwo {
                type Error = anyhow::Error;

                fn try_from(pow: u16) -> anyhow::Result<Self> {
                    TryFrom::<u64>::try_from(pow as u64)
                }
            }

            impl TryFrom<u32> for PowerOfTwo {
                type Error = anyhow::Error;

                fn try_from(pow: u32) -> anyhow::Result<Self> {
                    TryFrom::<u64>::try_from(pow as u64)
                }
            }

            impl TryFrom<u64> for PowerOfTwo
            {
                type Error = anyhow::Error;

                fn try_from(pow: u64) -> anyhow::Result<Self> {
                    $(
                        const [<P $pow>]: u64 = 1 << $pow;
                    )*

                    let pow: u64 = pow.into();
                    match pow {
                        0 => anyhow::Ok(Self::ZERO),
                        $(
                            [<P $pow>] => anyhow::Ok(Self::[<P $pow>]),
                        )*
                        _ => anyhow::bail!("Not a power of two: {pow}"),
                    }
                }
            }
        }
    };
}

power_of_two!(
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63
);

/// The core [Service]s available in Madara.
///
/// Note that [PowerOfTwo::ZERO] represents [MadaraServiceId::Monitor] as
/// [ServiceMonitor] is always running and therefore is the genesis state of all
/// other services.
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
    P2p,
    Analytics,
}

impl ServiceId for MadaraServiceId {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        match self {
            MadaraServiceId::Monitor => PowerOfTwo::ZERO,
            MadaraServiceId::Database => PowerOfTwo::P0,
            MadaraServiceId::L1Sync => PowerOfTwo::P1,
            MadaraServiceId::L2Sync => PowerOfTwo::P2,
            MadaraServiceId::BlockProduction => PowerOfTwo::P3,
            MadaraServiceId::RpcUser => PowerOfTwo::P4,
            MadaraServiceId::RpcAdmin => PowerOfTwo::P5,
            MadaraServiceId::Gateway => PowerOfTwo::P6,
            MadaraServiceId::Telemetry => PowerOfTwo::P7,
            MadaraServiceId::P2p => PowerOfTwo::P8,
            MadaraServiceId::Analytics => PowerOfTwo::P9,
        }
    }
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
                Self::P2p => "p2p",
                Self::Analytics => "analytics",
            }
        )
    }
}

impl std::ops::BitOr for MadaraServiceId {
    type Output = u64;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.svc_id() as u64 | rhs.svc_id() as u64
    }
}

impl std::ops::BitAnd for MadaraServiceId {
    type Output = u64;

    fn bitand(self, rhs: Self) -> Self::Output {
        self.svc_id() as u64 & rhs.svc_id() as u64
    }
}

impl From<PowerOfTwo> for MadaraServiceId {
    fn from(value: PowerOfTwo) -> Self {
        match value {
            PowerOfTwo::ZERO => Self::Monitor,
            PowerOfTwo::P0 => Self::Database,
            PowerOfTwo::P1 => Self::L1Sync,
            PowerOfTwo::P2 => Self::L2Sync,
            PowerOfTwo::P3 => Self::BlockProduction,
            PowerOfTwo::P4 => Self::RpcUser,
            PowerOfTwo::P5 => Self::RpcAdmin,
            PowerOfTwo::P6 => Self::Gateway,
            PowerOfTwo::P7 => Self::Telemetry,
            PowerOfTwo::P8 => Self::P2p,
            PowerOfTwo::P9 => Self::Analytics,
            _ => Self::Telemetry,
        }
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
pub struct MadaraServiceMask(std::sync::atomic::AtomicU64);

impl MadaraServiceMask {
    #[cfg(feature = "testing")]
    pub fn new_for_testing() -> Self {
        Self(std::sync::atomic::AtomicU64::new(u64::MAX))
    }

    #[inline(always)]
    pub fn status(&self, svc: impl ServiceId) -> MadaraServiceStatus {
        (self.value() & svc.svc_id() as u64 > 0).into()
    }

    #[inline(always)]
    pub fn is_active_some(&self) -> bool {
        self.value() > 0
    }

    #[inline(always)]
    pub fn activate(&self, svc: impl ServiceId) -> MadaraServiceStatus {
        let prev = self.0.fetch_or(svc.svc_id() as u64, std::sync::atomic::Ordering::SeqCst);
        (prev & svc.svc_id() as u64 > 0).into()
    }

    #[inline(always)]
    pub fn deactivate(&self, svc: impl ServiceId) -> MadaraServiceStatus {
        let svc = svc.svc_id() as u64;
        let prev = self.0.fetch_and(!svc, std::sync::atomic::Ordering::SeqCst);
        (prev & svc > 0).into()
    }

    fn active_set(&self) -> Vec<MadaraServiceId> {
        let mut i = MadaraServiceId::Telemetry.svc_id() as u64;
        let state = self.value();
        let mut set = Vec::with_capacity(SERVICE_COUNT_MAX);

        while i > 0 {
            let mask = state & i;

            if mask > 0 {
                let pow = PowerOfTwo::try_from(mask).expect("mask is a power of 2");
                set.push(MadaraServiceId::from(pow));
            }

            i >>= 1;
        }

        set
    }

    fn value(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
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
    id: PowerOfTwo,
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
            service_update_sender: Arc::new(tokio::sync::broadcast::channel(SERVICE_COUNT_MAX).0),
            service_update_receiver: None,
            id: MadaraServiceId::Monitor.svc_id(),
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
        Self { services: Arc::new(MadaraServiceMask::new_for_testing()), ..Default::default() }
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
    /// [ServiceContext] is cancelled.
    ///
    /// This allows for more manual implementation of cancellation logic than
    /// [ServiceContext::run_until_cancelled], and should only be used in cases
    /// where using `run_until_cancelled` is not possible or would be less
    /// clear.
    ///
    /// A service is cancelled after calling [ServiceContext::cancel_local],
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
            // been cancelled or this service was deactivated
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

    /// Checks if the service associated to this [ServiceContext] was cancelled.
    ///
    /// This happens after calling [ServiceContext::cancel_local],
    /// [ServiceContext::cancel_global] or [ServiceContext::service_remove].
    ///
    /// # Limitations
    ///
    /// This function should _not_ be used when waiting on potentially
    /// blocking futures which can be cancelled without entering an invalid
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
    /// in an invalid state if cancelled, then you should be using
    /// [ServiceContext::cancelled] instead.
    #[inline(always)]
    pub fn is_cancelled(&self) -> bool {
        self.token_global.is_cancelled()
            || self.token_local.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
            || self.services.status(self.id) == MadaraServiceStatus::Off
    }

    /// Runs a [Future] until the [Service] associated to this [ServiceContext]
    /// is cancelled.
    ///
    /// This happens after calling [ServiceContext::cancel_local],
    /// [ServiceContext::cancel_global] or [ServiceContext::service_remove].
    ///
    /// # Cancellation safety
    ///
    /// It is important that the future you pass to this function is _cancel-
    /// safe_ as it will be forcefully shutdown if ever the service is cancelled.
    /// This means your future might be interrupted at _any_ point in its
    /// execution.
    ///
    /// Futures can be considered as cancel-safe in the context of Madara if
    /// their computation can be interrupted at any point without causing any
    /// side-effects to the running node.
    ///
    /// # Returns
    ///
    /// The return value of the future wrapped in [Some], or [None] if the
    /// service was cancelled.
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

    /// The id of the [Service] associated to this [ServiceContext]
    pub fn id(&self) -> PowerOfTwo {
        self.id
    }

    /// Sets the id of this [ServiceContext]
    pub fn with_id(mut self, id: impl ServiceId) -> Self {
        self.id = id.svc_id();
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

    /// Atomically checks if a [Service] is running.
    #[inline(always)]
    pub fn service_status(&self, svc: impl ServiceId) -> MadaraServiceStatus {
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
    pub fn service_add(&self, id: impl ServiceId) -> MadaraServiceStatus {
        let svc_id = id.svc_id();
        let res = self.services.activate(id);

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
    pub fn service_remove(&self, id: impl ServiceId) -> MadaraServiceStatus {
        let svc_id = id.svc_id();
        let res = self.services.deactivate(id);
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
        self.services.status(self.id)
    }
}

/// Provides info about a [Service]'s status.
///
/// Used as part of [ServiceContext::service_subscribe].
#[derive(Clone, Copy)]
pub struct ServiceTransport {
    pub svc_id: PowerOfTwo,
    pub status: MadaraServiceStatus,
}

/// A microservice in the Madara node.
///
/// The app is divided into services, with each service handling different
/// responsibilities within the app. Depending on the startup configuration,
/// some services are enabled and some are disabled.
///
/// Services should be started with [ServiceRunner::service_loop].
///
/// # Writing your own service
///
/// Writing a service involves two main steps:
///
/// 1. Implementing the [Service] trait
/// 2. Implementing the [ServiceId] trait
///
/// It is also recommended you create your own enum for storing service ids
/// which itself implements [ServiceId]. This helps keep your code organized as
/// [PowerOfTwo::P17] does not have much meaning in of itself.
///
/// ```rust
/// # use mp_utils::service::Service;
/// # use mp_utils::service::ServiceId;
/// # use mp_utils::service::PowerOfTwo;
/// # use mp_utils::service::ServiceRunner;
/// # use mp_utils::service::ServiceMonitor;
///
/// // This enum only exist to make it easier for us to remember which
/// // PowerOfTwo represents our services.
/// pub enum MyServiceId {
///     MyServiceA,
///     MyServiceB
/// }
///
/// impl ServiceId for MyServiceId {
///     #[inline(always)]
///     fn svc_id(&self) -> PowerOfTwo {
///         match self {
///             // PowerOfTwo::P0 up until PowerOfTwo::P7 are already in use by
///             // MadaraServiceId, you should not use them!
///             Self::MyServiceA => PowerOfTwo::P8,
///             Self::MyServiceB => PowerOfTwo::P9,
///         }
///     }
/// }
///
/// // Similarly, this enum is more explicit for our usecase than Option<T>
/// #[derive(Clone, Debug)]
/// pub enum Channel<T: Sized + Send + Sync> {
///     Open(T),
///     Closed
/// }
///
/// // An example service, sends over 4 integers to `ServiceB` and the exits
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
///                 const SLEEP: std::time::Duration = std::time::Duration::from_secs(1);
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
///             // `Channel` enum.
///             sx.send(Channel::Closed);
///
///             anyhow::Ok(())
///         });
///
///         anyhow::Ok(())
///     }
/// }
///
/// impl ServiceId for MyServiceA {
///     fn svc_id(&self) -> PowerOfTwo {
///         MyServiceId::MyServiceA.svc_id()
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
///         });
///
///         anyhow::Ok(())
///     }
/// }
///
/// impl ServiceId for MyServiceB {
///     fn svc_id(&self) -> PowerOfTwo {
///         MyServiceId::MyServiceB.svc_id()
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
///     let monitor = ServiceMonitor::default()
///         .with(service_a)?
///         .with(service_b)?;
///
///     // We can use `MyServiceId` directly here. Most service methods only
///     // require an `impl ServiceId`, so this kind of pattern is very much
///     // recommended.
///     monitor.activate(MyServiceId::MyServiceA);
///     monitor.activate(MyServiceId::MyServiceB);
///
///     monitor.start().await?;
///
///     anyhow::Ok(())
/// }
/// ```
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync + ServiceId {
    /// Default impl does not start any task.
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Allows a [Service] to identify itself
///
/// Services are identified using a unique [PowerOfTwo]
pub trait ServiceId {
    fn svc_id(&self) -> PowerOfTwo;
}

#[async_trait::async_trait]
impl Service for Box<dyn Service> {
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        self.as_mut().start(_runner).await
    }
}

impl ServiceId for Box<dyn Service> {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        self.as_ref().svc_id()
    }
}

/// Wrapper around a [tokio::task::JoinSet] and a [ServiceContext].
///
/// Used to enforce certain shutdown behavior onto [Service]s which are started
/// with [ServiceRunner::service_loop]
pub struct ServiceRunner<'a> {
    ctx: ServiceContext,
    join_set: &'a mut JoinSet<anyhow::Result<PowerOfTwo>>,
}

impl<'a> ServiceRunner<'a> {
    fn new(ctx: ServiceContext, join_set: &'a mut JoinSet<anyhow::Result<PowerOfTwo>>) -> Self {
        Self { ctx, join_set }
    }

    /// The main loop of a [Service].
    ///
    /// The future passed to this function should complete _only once the
    /// service completes or is cancelled_. Services that complete early will
    /// automatically be cancelled.
    ///
    /// > **Caution**
    /// > As a safety mechanism, services have up to [SERVICE_GRACE_PERIOD]
    /// > to gracefully shutdown before they are forcefully cancelled. This
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
            if id != MadaraServiceId::Monitor.svc_id() {
                tracing::debug!("Starting service with id: {id}");
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

            if id != MadaraServiceId::Monitor.svc_id() {
                tracing::debug!("Shutting down service with id: {id}");
            }

            Ok(id)
        });
    }

    async fn stopper(mut ctx: ServiceContext, id: &PowerOfTwo) {
        ctx.cancelled().await;
        tokio::time::sleep(SERVICE_GRACE_PERIOD).await;

        tracing::warn!("⚠️  Forcefully shutting down service: {}", MadaraServiceId::from(*id));
    }
}

pub struct ServiceMonitor {
    services: [Option<Box<dyn Service>>; SERVICE_COUNT_MAX],
    join_set: JoinSet<anyhow::Result<PowerOfTwo>>,
    status_request: Arc<MadaraServiceMask>,
    status_actual: Arc<MadaraServiceMask>,
}

impl Default for ServiceMonitor {
    fn default() -> Self {
        Self {
            services: [const { None }; SERVICE_COUNT_MAX],
            join_set: JoinSet::new(),
            status_request: Arc::default(),
            status_actual: Arc::default(),
        }
    }
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
        let idx = svc.svc_id().index();
        self.services[idx] = match self.services[idx] {
            Some(_) => anyhow::bail!("Services has already been added"),
            None => Some(Box::new(svc)),
        };

        anyhow::Ok(self)
    }

    /// Marks a [Service] as active, meaning it will be started automatically
    /// when calling [ServiceMonitor::start].
    pub fn activate(&self, id: impl ServiceId) {
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
                Some(svc) if self.status_request.status(svc.svc_id()) == MadaraServiceStatus::On => {
                    let id = svc.svc_id();
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
                            tracing::debug!("Service {id} has shut down");
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
                            if self.status_actual.status(svc_id) == MadaraServiceStatus::Off {
                                self.status_actual.activate(svc_id);

                                let ctx = ctx.child().with_id(svc_id);
                                let runner = ServiceRunner::new(ctx, &mut self.join_set);
                                svc.start(runner)
                                    .await
                                    .context("Starting service")?;

                                tracing::debug!("Service {svc_id} has started");
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
