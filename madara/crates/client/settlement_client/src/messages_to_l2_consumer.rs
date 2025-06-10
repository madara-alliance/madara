use crate::{client::SettlementClientTrait, messaging::check_message_to_l2_validity};
use anyhow::Context;
use mc_db::MadaraBackend;
use mp_transactions::L1HandlerTransactionWithFee;
use std::sync::Arc;
use tokio::sync::Notify;

pub struct MessagesToL2Consumer {
    next_nonce: u64,
    backend: Arc<MadaraBackend>,
    settlement_client: Arc<dyn SettlementClientTrait>,
    notify: Arc<Notify>,
}

impl MessagesToL2Consumer {
    pub async fn consume_next_or_wait(&mut self) -> anyhow::Result<L1HandlerTransactionWithFee> {
        // Avoid missing messages: we subscribe to the notify before checking the db. This ensure we dont miss a message
        // when it is added just between when we checked db and subscribed.
        // [[(perf) 0) Optimistically query the db]]
        // 1) Subscribe to notifications
        // 2) Query the db
        // 3) Wait on the subscription
        let permit = self.notify.notified(); // This doesn't actually register us to be notified yet.
        tokio::pin!(permit);
        let mut wait = false;
        loop {
            // Register us to be notified. First loop doesn't subscribe, optimistic query.
            if wait {
                permit.as_mut().enable();
            }

            // Return what's in db. Skip and remove invalid/cancelled messages.
            while let Some(msg) = self
                .backend
                .get_next_pending_message_to_l2(self.next_nonce)
                .context("Getting next pending message to l2")?
            {
                self.next_nonce = msg.tx.nonce + 1;
                match check_message_to_l2_validity(&self.settlement_client, &self.backend, &msg)
                    .await
                    .context("Checking message to l2 validity")?
                {
                    // can be consumed (not cancelled, still exists, not already in chain)
                    true => return Ok(msg),
                    false => self
                        .backend
                        .remove_pending_message_to_l2(msg.tx.nonce)
                        .context("Removing pending message to l2")?,
                }
            }

            // We have no more messages in db.

            if wait {
                permit.as_mut().await; // Wait until we're notified.
                permit.set(self.notify.notified());
            }
            wait = true;
        }
    }
}
