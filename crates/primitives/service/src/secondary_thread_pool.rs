use anyhow::Context;
use std::{borrow::Cow, sync::Arc, thread};
use tokio::sync::oneshot;

use crate::service::{Service, TaskGroup};
/// Wraps a service or service group into an isolated single-thread tokio runtime.
pub struct IsolatedThreadService {
    inner: Option<Box<dyn Service>>,
    thread: Arc<std::sync::Mutex<Option<thread::JoinHandle<anyhow::Result<()>>>>>,
    name: Cow<'static, str>,
}

impl IsolatedThreadService {
    pub fn new(inner: impl Service) -> Self {
        Self {
            name: inner.name().to_owned().into(),
            inner: Some(Box::new(inner)),
            thread: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

impl Drop for IsolatedThreadService {
    fn drop(&mut self) {
        if let Some(handle) = self.thread.lock().expect("poisoned lock").take() {
            let _res = handle.join();
        }
    }
}

#[async_trait::async_trait]
impl Service for IsolatedThreadService {
    async fn start(&mut self, task_group: &mut TaskGroup) -> anyhow::Result<()> {
        let (sender, receiver) = oneshot::channel();

        let mut inner = self.inner.take().expect("Service already started");

        let mut inner_fn = move || {
            let rt =
                tokio::runtime::Builder::new_current_thread().enable_all().build().context("Creating tokio runtime")?;
            rt.block_on(inner.start_and_drive_to_end())
        };

        // Use drop as this runs when unwinding the stack too.
        struct ChannelGuard(Option<oneshot::Sender<()>>);
        impl Drop for ChannelGuard {
            fn drop(&mut self) {
                let _res = self.0.take().map(|c| c.send(()));
            }
        }

        let thread = thread::Builder::new()
            .name(self.name().into())
            .spawn(move || {
                let _guard = ChannelGuard(Some(sender));
                inner_fn()
            })
            .with_context(|| format!("Spawning thread for isolated thread service: {}", self.name()))?;

        if self.thread.lock().expect("poisoned lock").replace(thread).is_some() {
            panic!("Service already started");
        }

        let thread_handle = Arc::clone(&self.thread);
        task_group.spawn(async move {
            let _res = receiver.await;
            let Some(handle) = thread_handle.lock().expect("poisoned lock").take() else { return Ok(()) };

            match handle.join() {
                Ok(res) => res,
                Err(panic) => std::panic::resume_unwind(panic),
            }
        });

        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}
