use std::{future::Future, pin::Pin, task};
use tokio::{sync::oneshot, task::JoinHandle};

pub mod crypto;
pub mod hash;
pub mod parsers;
pub mod rayon;
pub mod serde;
pub mod service;

pub use hash::trim_hash;

#[track_caller] // forward the tokio track_caller
pub fn spawn<F: Future + Send + 'static>(future: F) -> AbortOnDrop<F::Output>
where
    F::Output: Send + 'static,
{
    AbortOnDrop::spawn(future)
}

#[track_caller]
pub fn spawn_blocking<F: FnOnce() -> R + Send + 'static, R: Send + 'static>(f: F) -> AbortOnDrop<R> {
    AbortOnDrop::spawn_blocking(f)
}

/// This ensures structural-concurrency. Use this when you know the task is cancellation-safe, it is fine to just
/// drop the futures. Otherwise, you will need to use a graceful abort signal.
pub struct AbortOnDrop<T>(JoinHandle<T>);
impl<T: Send + 'static> AbortOnDrop<T> {
    #[track_caller] // forward the tokio track_caller
    pub fn spawn<F: Future<Output = T> + Send + 'static>(future: F) -> Self {
        Self(tokio::spawn(future))
    }

    #[track_caller]
    pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static>(f: F) -> Self {
        Self(tokio::task::spawn_blocking(f))
    }
}
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort()
    }
}
impl<T> Future for AbortOnDrop<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        // Panic: the task is never aborted, except on drop in which case it cannot be polled again.
        Pin::new(&mut self.get_mut().0).poll(cx).map(|r| r.expect("Join error"))
    }
}
impl<T> From<JoinHandle<T>> for AbortOnDrop<T> {
    fn from(value: JoinHandle<T>) -> Self {
        Self(value)
    }
}

#[derive(Debug, Default)]
pub struct StopHandle(Option<oneshot::Sender<()>>);

impl StopHandle {
    pub fn new(inner: Option<oneshot::Sender<()>>) -> Self {
        Self(inner)
    }
}
impl Drop for StopHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _res = sender.send(());
        }
    }
}
