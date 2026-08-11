//! Madara block production. This crate is responsible for _producing state_ when the node is
//! running as a sequencer. Full node sync does _not_ use this crate. Instead, refer to [`mc_sync`].
//!
//! # Execution Model
//!
//! Block production works by processing transactions which are streamed to it from the [mempool].
//! For performance reasons, this is separated into a **batching phase**, an **aggregation phase**
//! and a **pending phase**, which are optimistically parallelized. Effectively, this allows block
//! production to lag behind on certain tasks (on large blocks for example) while still being able
//! to make progress wherever it can.
//!
//! ## Batching Phase
//!
//! Because efficient block production is complicated (help my head hurts), transaction aggregation
//! is handled by a [`Batcher`] which consumes three transaction streams (this has the disadvantage
//! of making the code quite hard to follow at times :/ ).
//!
//! - The `l1_tx_stream` sends all l1 transactions received as part of l1 sync (this is different
//!   from l2 sync, l1 sync is always active even during block production because we need a way to
//!   register l1 to l2 messages even as we are producing new blocks).
//!
//! - The `mempool_tx_stream` sends all validated transaction which have been added to the mempool,
//!   following whichever mempool ordering policy is in use at the time of block production (this
//!   can be first-come-first-served or fee-market).
//!
//! - The `bypass_txs_stream` allows transactions to be added to block production _without having
//!   them be validated by the mempool_. This is used by certain _permisioned_ (admin) endpoints and
//!   is useful when initially setting up a chain, where trying to deploy some genesis contracts
//!   might result in an invalidation until they have been deployed. This kind of cyclical
//!   invalidation requires a way to force-add transactions, and the bypass stream does just that.
//!
//! The batcher aggregates transactions from all these streams into a 'batch', which it sends back
//! to the main block production task via message passing using channels, where the sending end
//! is [`ExecutorThreadHandle::send_batch`] and the receiving end is
//! [`ExecutorThread::incoming_batches`].
//!
//! ## Aggregation Phase
//!
//! Between transaction batches, updates to the block production state is handled by the
//! [`BlockProductionTask`], which is responsible for starting, running and monitoring block
//! production. The aggregation phase is used to drive updates to the block production state through
//! an actor model implemented via message passing, where the block production task drives itself to
//! completion by messaging itself across threads, state updates and method calls. This is handled
//! by [`process_reply`], which needs to handle the following messages:
//!
//! - [`StartNewBlock`]: this message is sent whenever the [`ExecutorThread`] starts a new block and
//!   it instructs the [`BlockProductionTask`] to clear its [`PendingBlockState`] in preparation for
//!   the next batch.
//!
//! - [`BatchExecuted`]: this message is sent whenever the [`ExecutorThread`] has finished executing
//!   a batch, marking it as ready to consume by the [`BlockProductionTask`]. When this is received,
//!   the latest batch is added to the [`PendingBlockState`].
//!
//! - [`EndBlock`]: this message is sent by the [`ExecutorThread`] under one of several condition.
//!   Either the block has been forcefully closed (for example by an admin endpoint), or it is full
//!   as per the constraints set it the chain config, else the block time has elapsed as per the
//!   constraints set in the chain config. In any of these cases, whenever the [`ExecutorThread`]
//!   receives this message it will proceed to finalize (seal) the pending block and store it to db
//!   as a full block.
//!
//! - [`EndFinalBlock`]: sent during graceful shutdown when batch channel closes. Contains
//!   `Some(summary)` to close an existing block, or `None` to signal completion without a block.
//!
//! ## Pending Phase
//!
//! The [`PendingBlockState`] is primarily kept in RAM but is also flushed to the database as a
//! **pre-confirmed block**. This ensures that if the node crashes or restarts during block
//! production, we can recover the work done so far.
//!
//! Currently, we flush the pending block state to the database whenever a new batch of transactions
//! is executed (`BatchExecuted` message). This persistence allows us to recover the pre-confirmed
//! block upon restart.
//!
//! ### Restart Recovery
//!
//! When Madara starts, it checks for an existing pre-confirmed block. If found:
//! 1. It loads the saved **runtime execution configuration** (ChainConfig, VersionedConstants, etc.)
//!    to ensure re-execution uses the exact same parameters as the original execution.
//! 2. It **re-executes** all transactions in the pre-confirmed block. This is necessary because
//!    intermediate execution artifacts (like bouncer weights and state diffs) are not fully persisted.
//! 3. It closes the block immediately, effectively resuming the chain from where it left off.
//!
//! This mechanism guarantees deterministic recovery under the saved header/runtime context even if
//! the node's configuration changes between restarts (e.g., toggling fee charging).
//!
//! ## Graceful Shutdown and Error Handling
//!
//! The [`BlockProductionTask::run`] method implements graceful shutdown and error handling for
//! batcher and executor tasks. The main loop tracks completion of both tasks, which only complete
//! during shutdown scenarios (cancellation, error, or panic).
//!
//! ### Graceful Shutdown
//!
//! When a cancellation signal is received:
//! 1. The batcher detects cancellation and exits gracefully, closing the `send_batch` channel
//! 2. The executor detects the channel closure and finalizes any open block
//! 3. The executor sends an `EndFinalBlock` message (shutdown-specific) and then completes
//! 4. The main loop processes the `EndFinalBlock`, closes the block, and exits when both tasks complete
//!
//! ### Batcher Panic/Error
//!
//! If the batcher encounters an error or panics:
//! - **With preconfirmed block**: The error is saved and graceful shutdown is attempted. The batcher
//!   closes the channel, executor closes the block, and shutdown completes with the saved error.
//! - **Without preconfirmed block**: The error is returned immediately (no need to wait for executor).
//!
//! ### Executor Panic
//!
//! If the executor thread panics:
//! - The panic is caught and propagated via the `stop` channel
//! - The main loop resumes the panic, causing the block to remain preconfirmed
//! - The preconfirmed block will be handled on restart
//!
//! The loop exits when:
//! - Batcher completed AND `EndFinalBlock` was processed -> returns `Ok(())` or saved batcher error
//!
//! [mempool]: mc_mempool
//! [`StartNewBlock`]: executor::ExecutorMessage::StartNewBlock
//! [`BatchExecuted`]: executor::ExecutorMessage::BatchExecuted
//! [`EndBlock`]: executor::ExecutorMessage::EndBlock
//! [`EndFinalBlock`]: executor::ExecutorMessage::EndFinalBlock
//! [`ExecutorThreadHandle::send_batch`]: executor::ExecutorThreadHandle::send_batch
//! [`ExecutorThread::incoming_batches`]: executor::thread::ExecutorThread::incoming_batches
//! [`ExecutorThread`]: executor::thread::ExecutorThread
//! [`process_reply`]: BlockProductionTask::process_reply

mod batcher;
mod classifier;
mod close_queue;
pub(crate) mod comparator;
mod executor;
pub mod fallback;
mod finalizer;
mod handle;
pub mod metrics;
pub(crate) mod reexecution;
mod task;
mod util;

pub use handle::BlockProductionHandle;
pub use task::{BlockProductionStateNotification, BlockProductionTask};
pub(crate) use task::{CurrentBlockState, MempoolIntakeMode};
pub use util::RustExecRuntimeOptions;

#[cfg(test)]
pub(crate) use task::tests;
