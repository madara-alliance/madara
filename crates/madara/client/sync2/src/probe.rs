use futures::future::BoxFuture;
use futures::FutureExt;
use std::future::Future;
use std::time::Duration;
use tokio::time::Instant;

type ProbeFuture<T> = BoxFuture<'static, anyhow::Result<Option<T>>>;

pub struct ProbeState<T: Clone> {
    last_val: Option<T>,
    future: Option<ProbeFuture<T>>,
    make_future: Box<dyn FnMut(Option<T>) -> ProbeFuture<T> + Send>,
    wait: Option<Instant>,
    wait_duration: Duration,
}

impl<T: Clone> ProbeState<T> {
    pub fn new<F, Fut>(mut f: F, wait_duration: Duration) -> Self
    where
        F: FnMut(Option<T>) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<Option<T>>> + Send + 'static,
    {
        Self { last_val: None, future: None, make_future: Box::new(move |v| f(v).boxed()), wait_duration, wait: None }
    }

    pub async fn run(&mut self) -> anyhow::Result<Option<T>> {
        if let Some(wait) = self.wait {
            tokio::time::sleep_until(wait).await;
            self.wait = None;
        }
        let fut = self.future.get_or_insert_with(|| (self.make_future)(self.last_val.clone()));
        let res = fut.await;
        self.future = None;
        let res = res?;
        self.wait = Some(Instant::now() + self.wait_duration);

        self.last_val = res.clone();
        Ok(res)
    }

    pub fn last_val(&self) -> Option<T> {
        self.last_val.clone()
    }
}
