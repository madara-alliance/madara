use crate::{client::SettlementLayerProvider, messaging::check_message_to_l2_validity};
use anyhow::Context;
use mc_db::MadaraBackend;
use mp_transactions::L1HandlerTransactionWithFee;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub struct MessagesToL2Consumer {
    next_nonce: u64,
    /// Newly queued nonces from backfill and live sync can be revisited below the forward cursor.
    newly_queued_nonces: Arc<Mutex<BTreeSet<u64>>>,
    backend: Arc<MadaraBackend>,
    l1_read: Arc<dyn SettlementLayerProvider>,
    notify: Arc<Notify>,
    unsafe_skip_l1_message_consumed_check: bool,
}

impl MessagesToL2Consumer {
    pub fn new(
        backend: Arc<MadaraBackend>,
        l1_read: Arc<dyn SettlementLayerProvider>,
        notify: Arc<Notify>,
        newly_queued_nonces: Arc<Mutex<BTreeSet<u64>>>,
        unsafe_skip_l1_message_consumed_check: bool,
    ) -> Self {
        Self { next_nonce: 0, newly_queued_nonces, backend, l1_read, notify, unsafe_skip_l1_message_consumed_check }
    }

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

            // A new message below next_nonce needs one targeted rewind, without rereading every
            // in-flight row on each ordinary message.
            let late_nonce = self
                .newly_queued_nonces
                .lock()
                .expect("backfill nonce mutex poisoned")
                .first()
                .copied()
                .filter(|nonce| *nonce < self.next_nonce);
            if let Some(nonce) = late_nonce {
                if let Some(msg) =
                    self.backend.get_pending_message_to_l2(nonce).context("Getting newly queued message")?
                {
                    match check_message_to_l2_validity(
                        &self.l1_read,
                        &self.backend,
                        &msg,
                        self.unsafe_skip_l1_message_consumed_check,
                    )
                    .await
                    .context("Checking newly queued message validity")?
                    {
                        true => {
                            self.newly_queued_nonces.lock().expect("backfill nonce mutex poisoned").remove(&nonce);
                            return Ok(msg);
                        }
                        false => {
                            self.backend.remove_pending_message_to_l2(nonce).context("Removing newly queued message")?
                        }
                    }
                }
                self.newly_queued_nonces.lock().expect("backfill nonce mutex poisoned").remove(&nonce);
                continue;
            }

            // Normal traffic keeps its monotone cursor and reads each pending row once.
            while let Some(msg) = self
                .backend
                .get_next_pending_message_to_l2(self.next_nonce)
                .context("Getting next pending message to l2")?
            {
                let nonce = msg.tx.nonce;
                match check_message_to_l2_validity(
                    &self.l1_read,
                    &self.backend,
                    &msg,
                    self.unsafe_skip_l1_message_consumed_check,
                )
                .await
                .context("Checking message to l2 validity")?
                {
                    // can be consumed (not cancelled, still exists, not already in chain)
                    true => {
                        self.next_nonce = nonce + 1;
                        self.newly_queued_nonces.lock().expect("backfill nonce mutex poisoned").remove(&nonce);
                        return Ok(msg);
                    }
                    false => {
                        self.backend.remove_pending_message_to_l2(nonce).context("Removing pending message to l2")?
                    }
                }
                self.next_nonce = nonce + 1;
                self.newly_queued_nonces.lock().expect("backfill nonce mutex poisoned").remove(&nonce);
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

#[cfg(test)]
mod tests {
    use crate::{
        client::{ClientType, MockSettlementLayerProvider},
        messages_to_l2_consumer::MessagesToL2Consumer,
    };
    use futures::FutureExt;
    use mc_db::MadaraBackend;
    use mockall::predicate;
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    fn l1_handler_tx(nonce: u64) -> L1HandlerTransactionWithFee {
        L1HandlerTransactionWithFee {
            tx: L1HandlerTransaction {
                version: Felt::ZERO,
                nonce,
                contract_address: Felt::TWO * Felt::from(nonce),
                entry_point_selector: Felt::ZERO,
                calldata: vec![Felt::THREE, Felt::ONE].into(),
            },
            paid_fee_on_l1: nonce as u128 * 3,
        }
    }

    fn mock_l1_handler_tx(mock: &mut MockSettlementLayerProvider, nonce: u64, is_pending: bool, has_cancel_req: bool) {
        mock.expect_calculate_message_hash()
            .with(predicate::eq(l1_handler_tx(nonce)))
            .returning(move |_| Ok(nonce.to_be_bytes().to_vec()));
        mock.expect_message_to_l2_has_cancel_request()
            .with(predicate::eq(nonce.to_be_bytes().to_vec()))
            .returning(move |_| Ok(has_cancel_req));
        mock.expect_message_to_l2_is_pending()
            .with(predicate::eq(nonce.to_be_bytes().to_vec()))
            .returning(move |_| Ok(is_pending));
    }

    #[test]
    fn test_consumer_checks_validity() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let mut mock = MockSettlementLayerProvider::new();
        mock.expect_get_client_type().returning(|| ClientType::Starknet);
        let notify = Arc::new(Notify::new());

        // nonce 4, is pending, not being cancelled, not consumed in db. => OK
        backend.write_pending_message_to_l2(&l1_handler_tx(4)).unwrap();
        mock_l1_handler_tx(&mut mock, 4, true, false);
        // nonce 5, is pending, not being cancelled, not consumed in db. => OK
        backend.write_pending_message_to_l2(&l1_handler_tx(5)).unwrap();
        mock_l1_handler_tx(&mut mock, 5, true, false);
        // nonce 7, is pending, not being cancelled, not consumed in db. => OK
        backend.write_pending_message_to_l2(&l1_handler_tx(7)).unwrap();
        mock_l1_handler_tx(&mut mock, 7, true, false);
        // nonce 3, not pending, not being cancelled, not consumed in db. => NOT OK
        backend.write_pending_message_to_l2(&l1_handler_tx(3)).unwrap();
        mock_l1_handler_tx(&mut mock, 3, false, false);
        // nonce 84, is pending, being cancelled, not consumed in db. => NOT OK
        backend.write_pending_message_to_l2(&l1_handler_tx(84)).unwrap();
        mock_l1_handler_tx(&mut mock, 84, true, true);
        // nonce 99, is pending, not being cancelled, consumed in db. => NOT OK
        backend.write_pending_message_to_l2(&l1_handler_tx(99)).unwrap();
        backend.write_l1_handler_txn_hash_by_nonce(99, &Felt::TWO).unwrap();
        mock_l1_handler_tx(&mut mock, 99, true, false);
        // nonce 103, is pending, not being cancelled, not consumed in db. => OK
        backend.write_pending_message_to_l2(&l1_handler_tx(103)).unwrap();
        mock_l1_handler_tx(&mut mock, 103, true, false);

        let mut consumer =
            MessagesToL2Consumer::new(backend.clone(), Arc::new(mock), notify, Default::default(), false);

        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(4));
        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(5));
        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(7));
        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(103));
        assert!(consumer.consume_next_or_wait().now_or_never().is_none()); // waiting.
    }

    #[test]
    fn test_consumer_waits() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let mut mock = MockSettlementLayerProvider::new();
        mock.expect_get_client_type().returning(|| ClientType::Starknet);
        let notify = Arc::new(Notify::new());

        mock_l1_handler_tx(&mut mock, 4, true, false);
        mock_l1_handler_tx(&mut mock, 5, true, false);

        let mut consumer =
            MessagesToL2Consumer::new(backend.clone(), Arc::new(mock), notify.clone(), Default::default(), false);

        // first: test empty, then write.
        {
            let mut fut = Box::pin(consumer.consume_next_or_wait());
            assert!(fut.as_mut().now_or_never().is_none()); // waiting.

            // fut is subscribed to the notify now.
            // Write and wake up.

            backend.write_pending_message_to_l2(&l1_handler_tx(4)).unwrap();
            notify.notify_waiters();

            // should be woken up and return tx.
            assert_eq!(fut.as_mut().now_or_never().unwrap().unwrap(), l1_handler_tx(4));
        }
        // Should be empty again
        {
            let mut fut = Box::pin(consumer.consume_next_or_wait());
            assert!(fut.as_mut().now_or_never().is_none()); // waiting.
        } // listener is dropped.

        // no one is listening, write
        backend.write_pending_message_to_l2(&l1_handler_tx(5)).unwrap();

        // Should return the msg.
        {
            assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(5));
        }
        // Should be empty again
        {
            let mut fut = Box::pin(consumer.consume_next_or_wait());
            assert!(fut.as_mut().now_or_never().is_none()); // waiting.
        }
    }

    #[test]
    fn test_consumer_picks_late_backfill_and_live_nonce_without_redelivering() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let notify = Arc::new(Notify::new());
        let mut mock = MockSettlementLayerProvider::new();
        mock.expect_get_client_type().returning(|| ClientType::Starknet);
        for nonce in [34091, 34092, 34093, 34094] {
            mock_l1_handler_tx(&mut mock, nonce, true, false);
        }
        backend.write_pending_message_to_l2(&l1_handler_tx(34092)).unwrap();
        backend.write_pending_message_to_l2(&l1_handler_tx(34094)).unwrap();
        let newly_queued_nonces = Arc::new(Mutex::new(BTreeSet::new()));
        let mut consumer = MessagesToL2Consumer::new(
            backend.clone(),
            Arc::new(mock),
            notify.clone(),
            newly_queued_nonces.clone(),
            false,
        );

        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(34092));
        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(34094));
        backend.write_pending_message_to_l2(&l1_handler_tx(34093)).unwrap();
        newly_queued_nonces.lock().unwrap().insert(34093);
        notify.notify_waiters();
        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(34093));

        // Live sync may queue a lower nonce after a backfill advanced the forward cursor.
        backend.write_pending_message_to_l2(&l1_handler_tx(34091)).unwrap();
        newly_queued_nonces.lock().unwrap().insert(34091);
        notify.notify_waiters();
        assert_eq!(consumer.consume_next_or_wait().now_or_never().unwrap().unwrap(), l1_handler_tx(34091));
        assert!(consumer.consume_next_or_wait().now_or_never().is_none());
    }
}
