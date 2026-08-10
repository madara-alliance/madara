use super::*;
use crate::executor::ExecutorMessage;

impl ExecutorThread {
    pub(super) fn send_forward_reply(
        &mut self,
        execution_epoch: u64,
        block_number: Option<u64>,
        message_kind: &'static str,
        message: ExecutorMessage,
    ) -> ForwardReplySendOutcome {
        let mut execution_epoch_rx = self.execution_epoch_rx.clone();
        loop {
            let observed_epoch = *execution_epoch_rx.borrow_and_update();
            if observed_epoch != execution_epoch {
                tracing::info!(
                    block_number = ?block_number,
                    message_kind,
                    message_epoch = execution_epoch,
                    observed_epoch,
                    "executor_dropped_stale_forward_reply_before_send"
                );
                return ForwardReplySendOutcome::DroppedStale;
            }

            let sender = self.replies_sender.clone();
            match self.wait_rt.block_on(async {
                tokio::select! {
                    biased;
                    changed = execution_epoch_rx.changed() => match changed {
                        Ok(()) => ForwardReplyReservation::EpochChanged(*execution_epoch_rx.borrow()),
                        Err(_) => ForwardReplyReservation::EpochWatchClosed(*execution_epoch_rx.borrow()),
                    },
                    result = sender.reserve_owned() => match result {
                        Ok(permit) => ForwardReplyReservation::Permit(permit),
                        Err(_) => ForwardReplyReservation::ChannelClosed,
                    }
                }
            }) {
                ForwardReplyReservation::Permit(permit) => {
                    permit.send(message);
                    return ForwardReplySendOutcome::Sent;
                }
                ForwardReplyReservation::EpochChanged(observed_epoch) => {
                    if observed_epoch != execution_epoch {
                        tracing::info!(
                            block_number = ?block_number,
                            message_kind,
                            message_epoch = execution_epoch,
                            observed_epoch,
                            "executor_dropped_stale_forward_reply_after_epoch_advance"
                        );
                        return ForwardReplySendOutcome::DroppedStale;
                    }
                }
                ForwardReplyReservation::EpochWatchClosed(observed_epoch) => {
                    tracing::info!(
                        block_number = ?block_number,
                        message_kind,
                        message_epoch = execution_epoch,
                        observed_epoch,
                        "executor_stopped_waiting_on_forward_reply_after_epoch_watch_closed"
                    );
                    return ForwardReplySendOutcome::DroppedStale;
                }
                ForwardReplyReservation::ChannelClosed => return ForwardReplySendOutcome::ChannelClosed,
            }
        }
    }
}
