use core::task;
use futures::Future;
use std::pin::Pin;
use tokio::task::JoinHandle;

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
