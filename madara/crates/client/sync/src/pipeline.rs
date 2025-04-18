use futures::{
    future::{BoxFuture, OptionFuture},
    stream::FuturesOrdered,
    Future, FutureExt, StreamExt,
};
use std::{collections::VecDeque, fmt, ops::Range, sync::Arc};

struct RetryInput<I> {
    block_range: Range<u64>,
    input: Vec<I>,
}

#[derive(Debug)]
pub enum ApplyOutcome<Output> {
    Success(Output),
    #[allow(unused)]
    Retry,
}

/// The parallel step will be called concurrently for many blocks at a time. Then,
/// the sequential step will be called one block at a time, in order.
///
/// Fetches are usually done in the parallel step, while the sequential step is usually
/// used to perform actions that may not be possible in parallel over many blocks, such as
/// updating the global trie.
pub trait PipelineSteps: Sync + Send + 'static {
    /// Input batch item for the whole pipeline, which means, input to the parallel step.
    type InputItem: Send + Sync + Clone;
    /// Intermediate type returned by the parallel step, input to the seuential step.
    type SequentialStepInput: Send + Sync;
    /// Output of the pipeline, which means, output of the sequential step.
    type Output: Send + Sync + Clone;

    fn parallel_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> impl Future<Output = anyhow::Result<Self::SequentialStepInput>> + Send;
    fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> impl Future<Output = anyhow::Result<ApplyOutcome<Self::Output>>> + Send;
}

/// The pipeline controller is used to drive and execute the [`PipelineSteps`].
pub struct PipelineController<S: PipelineSteps> {
    steps: Arc<S>,
    /// Every parallel step currently being run. Polling it will poll every future, it will return the results as FCFS.
    queue: FuturesOrdered<ParallelStepFuture<S>>,
    /// The currently being run sequential step. There can only be one at a time.
    applying: Option<SequentialStepFuture<S>>,
    parallelization: usize,
    batch_size: usize,
    /// Inputs to be scheduled next into the parallel step.
    next_inputs: VecDeque<S::InputItem>,
    next_block_n_to_batch: u64,
    last_applied_block_n: Option<u64>,
}

type ParallelStepFuture<S> = BoxFuture<
    'static,
    anyhow::Result<(<S as PipelineSteps>::SequentialStepInput, RetryInput<<S as PipelineSteps>::InputItem>)>,
>;
type SequentialStepFuture<S> = BoxFuture<
    'static,
    anyhow::Result<(ApplyOutcome<<S as PipelineSteps>::Output>, RetryInput<<S as PipelineSteps>::InputItem>)>,
>;

impl<S: PipelineSteps> PipelineController<S> {
    /// Batch size is the maximum number of blocks per single parallel/sequential step.
    /// Note that the pipeline may schedule batches smaller than that if it cannot schedule a batch of that size.
    /// `starting_block_n` is the first block that will be imported once the pipeline is running.
    pub fn new(steps: S, parallelization: usize, batch_size: usize, starting_block_n: u64) -> Self {
        Self {
            steps: Arc::new(steps),
            queue: Default::default(),
            parallelization,
            batch_size,
            applying: None,
            next_inputs: VecDeque::with_capacity(2 * batch_size),
            next_block_n_to_batch: starting_block_n,
            last_applied_block_n: starting_block_n.checked_sub(1),
        }
    }

    pub fn next_input_block_n(&self) -> u64 {
        self.next_block_n_to_batch + self.next_inputs.len() as u64
    }
    pub fn last_applied_block_n(&self) -> Option<u64> {
        self.last_applied_block_n
    }

    pub fn can_schedule_more(&self) -> bool {
        if self.queue.len() >= self.parallelization {
            return false;
        }
        let slots_remaining = self.parallelization - self.queue.len();
        self.next_inputs.len() <= slots_remaining * self.batch_size
    }

    pub fn is_empty(&self) -> bool {
        self.applying.is_none() && self.queue.is_empty() && self.next_inputs.is_empty()
    }
    fn queue_len(&self) -> usize {
        self.queue.len()
    }
    fn is_applying(&self) -> bool {
        self.applying.is_some()
    }

    fn make_parallel_step_future(&self, input: RetryInput<S::InputItem>) -> ParallelStepFuture<S> {
        let steps = Arc::clone(&self.steps);
        async move { steps.parallel_step(input.block_range.clone(), input.input.clone()).await.map(|el| (el, input)) }
            .boxed()
    }
    fn make_sequential_step_future(
        &self,
        input: S::SequentialStepInput,
        retry_input: RetryInput<S::InputItem>,
    ) -> SequentialStepFuture<S> {
        let steps = Arc::clone(&self.steps);
        async move { steps.sequential_step(retry_input.block_range.clone(), input).await.map(|el| (el, retry_input)) }
            .boxed()
    }

    fn schedule_new_batch(&mut self) {
        // make batch
        let size = usize::min(self.next_inputs.len(), self.batch_size);

        let new_next_input_block_n = self.next_block_n_to_batch + size as u64;
        let block_range = self.next_block_n_to_batch..new_next_input_block_n;
        self.next_block_n_to_batch = new_next_input_block_n;
        let input = self.next_inputs.drain(0..size).collect();
        self.queue.push_back(self.make_parallel_step_future(RetryInput { block_range, input }));
    }

    /// Push an item to be scheduled next. Only call this when [`Self::can_schedule_more`] is true, to support
    /// backpressure correctly.
    pub fn push(&mut self, block_range: Range<u64>, input: impl IntoIterator<Item = S::InputItem>) {
        let next_input_block_n = self.next_input_block_n();
        // Skip items that we have already handled.
        self.next_inputs
            .extend(input.into_iter().zip(block_range).skip_while(|(_, n)| next_input_block_n < *n).map(|(v, _)| v));
    }

    /// Pulls a batch of item. This runs the pipeline until an output is ready.
    pub async fn next(&mut self) -> Option<anyhow::Result<(Range<u64>, S::Output)>> {
        loop {
            while self.next_inputs.len() >= self.batch_size && self.queue.len() <= self.parallelization {
                // Prefer making full batches.
                self.schedule_new_batch();
            }
            if self.queue.is_empty() && !self.next_inputs.is_empty() {
                // We make a smaller batch when we have nothing to do, to ensure progress.
                self.schedule_new_batch();
            }

            tokio::select! {
                Some(res) = OptionFuture::from(self.applying.as_mut()) => {
                    self.applying = None;
                    match res {
                        Err(err) => return Some(Err(err)),
                        Ok((ApplyOutcome::Success(out), retry_input)) => {
                            if let Some(last) = retry_input.block_range.clone().last() {
                                self.last_applied_block_n = Some(last);
                            }
                            return Some(Ok((retry_input.block_range, out)));
                        }
                        Ok((ApplyOutcome::Retry, retry_input)) => self.queue.push_front(self.make_parallel_step_future(retry_input)),
                    }
                }
                Some(res) = self.queue.next(), if self.applying.is_none() => {
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

pub struct PipelineStatus {
    pub jobs: usize,
    pub applying: bool,
    pub latest_applied: Option<u64>,
}

impl<S: PipelineSteps> PipelineController<S> {
    /// Get the status of a pipeline, for pretty-printing.
    pub fn status(&self) -> PipelineStatus {
        PipelineStatus {
            jobs: self.queue_len(),
            applying: self.is_applying(),
            latest_applied: self.last_applied_block_n(),
        }
    }
}

impl fmt::Display for PipelineStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::util::fmt_option;

        write!(f, "{} [{}", fmt_option(self.latest_applied, "N"), self.jobs)?;
        if self.applying {
            write!(f, "+")?;
        }
        write!(f, "]")
    }
}
