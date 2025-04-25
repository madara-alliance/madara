use futures::Future;
use std::{fmt, pin::Pin, task};
use tokio::task::JoinHandle;

pub fn fmt_option(opt: Option<impl fmt::Display>, or_else: impl fmt::Display) -> impl fmt::Display {
    DisplayFromFn(move |f| if let Some(val) = &opt { val.fmt(f) } else { or_else.fmt(f) })
}

pub struct DisplayFromFn<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result>(pub F);
impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for DisplayFromFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0)(f)
    }
}

/// This ensures structural-concurrency. All of the tasks in this service are cancellation-safe, it is fine to just
/// drop the futures.
pub struct AbortOnDrop<T>(JoinHandle<T>);
impl<T: Send + 'static> AbortOnDrop<T> {
    #[track_caller] // forward the tokio track_caller
    pub fn spawn<F: Future<Output = T> + Send + 'static>(future: F) -> Self {
        Self(tokio::spawn(future))
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

pub struct ServiceStateSender<T>(Option<tokio::sync::mpsc::UnboundedSender<T>>);

impl<T> Default for ServiceStateSender<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> ServiceStateSender<T> {
    pub fn send(&self, val: T) {
        if let Some(sender) = &self.0 {
            let _res = sender.send(val);
        }
    }
}

#[cfg(test)]
pub fn service_state_channel<T>() -> (ServiceStateSender<T>, tokio::sync::mpsc::UnboundedReceiver<T>) {
    let (sender, recv) = tokio::sync::mpsc::unbounded_channel();
    (ServiceStateSender(Some(sender)), recv)
}
