use std::{collections::VecDeque, ops::Range, sync::Arc};

use futures::{
    future::{LocalBoxFuture, OptionFuture},
    stream::FuturesOrdered,
    FutureExt, StreamExt,
};

struct RetryInput<I> {
    block_range: Range<u64>,
    input: Vec<I>,
}

pub enum ApplyOutcome<Output> {
    Success(Output),
    Retry,
}

pub trait PipelineSteps: Sync + Send + 'static {
    type InputItem: Send + Sync + Clone;
    type SequentialStepInput: Send + Sync;
    type Output: Send + Sync + Clone;

    async fn parallel_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> anyhow::Result<Self::SequentialStepInput>;
    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>>;
}

pub struct PipelineController<S: PipelineSteps> {
    steps: Arc<S>,
    queue: FuturesOrdered<ParallelStepFuture<S>>,
    parallelization: usize,
    batch_size: usize,
    applying: Option<SequentialStepFuture<S>>,
    next_inputs: VecDeque<S::InputItem>,
    next_input_block_n: u64,
}

type ParallelStepFuture<S> = LocalBoxFuture<
    'static,
    anyhow::Result<(<S as PipelineSteps>::SequentialStepInput, RetryInput<<S as PipelineSteps>::InputItem>)>,
>;
type SequentialStepFuture<S> = LocalBoxFuture<
    'static,
    anyhow::Result<(ApplyOutcome<<S as PipelineSteps>::Output>, RetryInput<<S as PipelineSteps>::InputItem>)>,
>;

impl<S: PipelineSteps> PipelineController<S> {
    pub fn new(steps: S, parallelization: usize, batch_size: usize) -> Self {
        Self {
            steps: Arc::new(steps),
            queue: Default::default(),
            parallelization,
            batch_size,
            applying: None,
            next_inputs: VecDeque::with_capacity(2 * batch_size),
            next_input_block_n: 0,
        }
    }

    pub fn can_schedule_more(&self) -> bool {
        self.queue.len() < self.parallelization
    }

    fn make_parallel_step_future(&self, input: RetryInput<S::InputItem>) -> ParallelStepFuture<S> {
        let steps = Arc::clone(&self.steps);
        async move { steps.parallel_step(input.block_range.clone(), input.input.clone()).await.map(|el| (el, input)) }
            .boxed_local()
    }
    fn make_sequential_step_future(
        &self,
        input: S::SequentialStepInput,
        retry_input: RetryInput<S::InputItem>,
    ) -> SequentialStepFuture<S> {
        let steps = Arc::clone(&self.steps);
        async move { steps.sequential_step(retry_input.block_range.clone(), input).await.map(|el| (el, retry_input)) }
            .boxed_local()
    }

    fn schedule_new_batch(&mut self) {
        // make batch
        let new_next_input_block_n = self.next_input_block_n + self.batch_size as u64;
        let block_range = self.next_input_block_n..new_next_input_block_n;
        self.next_input_block_n = new_next_input_block_n;
        let input = self.next_inputs.drain(0..self.batch_size).collect();
        self.queue.push_back(self.make_parallel_step_future(RetryInput { block_range, input }));
    }

    pub fn push(&mut self, input: impl IntoIterator<Item = S::InputItem>) {
        self.next_inputs.extend(input);
        while !self.next_inputs.is_empty() && self.can_schedule_more() {
            self.schedule_new_batch();
        }
    }

    pub async fn next(&mut self) -> Option<anyhow::Result<(Range<u64>, S::Output)>> {
        loop {
            tokio::select! {
                Some(res) = OptionFuture::from(self.applying.as_mut()) => {
                    match res {
                        Err(err) => return Some(Err(err)),
                        Ok((ApplyOutcome::Success(out), retry_input)) => {
                            self.applying = None;
                            return Some(Ok((retry_input.block_range, out)));
                        }
                        Ok((ApplyOutcome::Retry, retry_input)) => self.queue.push_front(self.make_parallel_step_future(retry_input)),
                    }
                }
                Some(res) = self.queue.next() => {
                    match res {
                        Ok((input, retry_input)) => {
                            self.applying = Some(self.make_sequential_step_future(input, retry_input));
                        }
                        Err(err) => return Some(Err(err)),
                    }
                }
                else => return None,
            }
        }
    }
}
