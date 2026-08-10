use super::*;

impl ExecutorThread {
    pub(super) fn wait_take_tx_batch(
        &mut self,
        current_block_n: Option<u64>,
        deadline: Option<Instant>,
        should_wait: bool,
        prefer_commands: bool,
    ) -> WaitTxBatchOutcome {
        if prefer_commands {
            if let Ok(cmd) = self.commands.try_recv() {
                tracing::debug!(
                    "executor_command_received block_number={current_block_n:?} command={cmd:?} receive_mode=try_recv"
                );
                return WaitTxBatchOutcome::Command(cmd);
            }
            if let Ok(batch) = self.incoming_batches.try_recv() {
                tracing::debug!(
                    "executor_batch_received block_number={current_block_n:?} tx_count={} receive_mode=try_recv",
                    batch.total_len()
                );
                return WaitTxBatchOutcome::Batch(batch);
            }
        } else {
            if let Ok(batch) = self.incoming_batches.try_recv() {
                tracing::debug!(
                    "executor_batch_received block_number={current_block_n:?} tx_count={} receive_mode=try_recv",
                    batch.total_len()
                );
                return WaitTxBatchOutcome::Batch(batch);
            }

            if let Ok(cmd) = self.commands.try_recv() {
                tracing::debug!(
                    "executor_command_received block_number={current_block_n:?} command={cmd:?} receive_mode=try_recv"
                );
                return WaitTxBatchOutcome::Command(cmd);
            }
        }

        if !should_wait {
            return WaitTxBatchOutcome::Batch(Default::default());
        }

        let now = Instant::now();
        let deadline_remaining_ms =
            deadline.map(|deadline| deadline.saturating_duration_since(now).as_secs_f64() * 1000.0);
        if deadline_remaining_ms.is_none_or(|remaining| remaining > 1.0) {
            tracing::debug!(
                "executor_waiting_for_batch block_number={current_block_n:?} until_block_time_deadline={} deadline_remaining_ms={deadline_remaining_ms:?}",
                deadline.is_some()
            );
        }
        let wait_started = StdInstant::now();

        // nb: tokio has blocking_recv, but no blocking_recv_timeout? this kinda sucks :(
        // especially because they do have it implemented in send_timeout and internally, they just have not exposed the
        // function.
        // Should be fine, as we optimistically try_recv above and we should only hit this when we actually have to wait.
        // nb.2: use an async block here, as timeout_at needs a runtime to be available on creation.
        self.wait_rt.block_on(async {
            if prefer_commands {
                tokio::select! {
                    biased;
                    Some(cmd) = self.commands.recv() => {
                        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                        tracing::debug!(
                            "executor_command_received block_number={current_block_n:?} command={cmd:?} wait_ms={wait_ms} receive_mode=recv"
                        );
                        WaitTxBatchOutcome::Command(cmd)
                    }
                    _ = OptionFuture::from(deadline.map(tokio::time::sleep_until)) => {
                        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                        if wait_ms > 1.0 {
                            tracing::debug!(
                                "executor_batch_wait_timed_out block_number={current_block_n:?} wait_ms={wait_ms}"
                            );
                        }
                        WaitTxBatchOutcome::Batch(Default::default())
                    }
                    el = self.incoming_batches.recv() => match el {
                        Some(el) => {
                            let summary = summarize_routed_batch(&el);
                            tracing::debug!("Got new batch with {} transactions.", el.total_len());
                            tracing::debug!(
                                total_txs = el.total_len(),
                                execution_mode = ?el.execution_mode,
                                execution_epoch = el.execution_epoch,
                                first_tx_hash = summary.first_hash.as_deref().unwrap_or("-"),
                                first_tx_nonce = summary.first_nonce.as_deref().unwrap_or("-"),
                                last_tx_hash = summary.last_hash.as_deref().unwrap_or("-"),
                                last_tx_nonce = summary.last_nonce.as_deref().unwrap_or("-"),
                                "executor_received_routed_batch"
                            );
                            WaitTxBatchOutcome::Batch(el)
                        }
                        None => {
                            let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                            tracing::info!(
                                "executor_batch_channel_closed block_number={current_block_n:?} wait_ms={wait_ms}"
                            );
                            WaitTxBatchOutcome::Exit
                        }
                    }
                }
            } else {
                tokio::select! {
                    Some(cmd) = self.commands.recv() => {
                        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                        tracing::debug!(
                            "executor_command_received block_number={current_block_n:?} command={cmd:?} wait_ms={wait_ms} receive_mode=recv"
                        );
                        WaitTxBatchOutcome::Command(cmd)
                    }
                    _ = OptionFuture::from(deadline.map(tokio::time::sleep_until)) => {
                        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                        if wait_ms > 1.0 {
                            tracing::debug!(
                                "executor_batch_wait_timed_out block_number={current_block_n:?} wait_ms={wait_ms}"
                            );
                        }
                        WaitTxBatchOutcome::Batch(Default::default())
                    }
                    el = self.incoming_batches.recv() => match el {
                        Some(el) => {
                            let summary = summarize_routed_batch(&el);
                            tracing::debug!("Got new batch with {} transactions.", el.total_len());
                            tracing::debug!(
                                total_txs = el.total_len(),
                                execution_mode = ?el.execution_mode,
                                execution_epoch = el.execution_epoch,
                                first_tx_hash = summary.first_hash.as_deref().unwrap_or("-"),
                                first_tx_nonce = summary.first_nonce.as_deref().unwrap_or("-"),
                                last_tx_hash = summary.last_hash.as_deref().unwrap_or("-"),
                                last_tx_nonce = summary.last_nonce.as_deref().unwrap_or("-"),
                                "executor_received_routed_batch"
                            );
                            WaitTxBatchOutcome::Batch(el)
                        }
                        None => {
                            let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                            tracing::info!(
                                "executor_batch_channel_closed block_number={current_block_n:?} wait_ms={wait_ms}"
                            );
                            WaitTxBatchOutcome::Exit
                        }
                    }
                }
            }
        })
    }
}
