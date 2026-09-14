//! Backend helpers for service-owned storage which is not tied to a block view.
//! These methods deliberately delegate persistence to the configured storage implementation.

use super::*;

// Delegate these db reads/writes. These are related to specific services, and are not specific to a block view / the head projection writer handle.
impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Returns the last L1 block scanned by the messaging synchronization service.
    /// Absence means no durable synchronization cursor has been recorded.
    pub fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        self.db.get_l1_messaging_sync_tip()
    }
    /// Returns the external database retention cursor maintained by its service.
    /// The backend does not reinterpret or advance the stored cursor.
    pub fn get_external_db_retention_cursor(&self) -> Result<Option<u64>> {
        self.db.get_external_db_retention_cursor()
    }
    /// Reads the pending L1-to-L2 message for one core-contract nonce.
    /// Missing nonces return `None` rather than a storage error.
    pub fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.db.get_pending_message_to_l2(core_contract_nonce)
    }
    /// Finds the first pending L1-to-L2 message at or above `start_nonce`.
    /// The ordering and lookup semantics are supplied by the storage backend.
    pub fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.db.get_next_pending_message_to_l2(start_nonce)
    }
    /// Returns the L1 transaction hash which emitted the L1->L2 message for the given core contract nonce, if known.
    ///
    /// This is written by the settlement client during L1 messaging sync and is later used to answer
    /// `starknet_getMessagesStatus` without making L1 requests at RPC time.
    pub fn get_l1_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<mp_convert::L1TransactionHash>> {
        self.db.get_l1_txn_hash_by_nonce(core_contract_nonce)
    }
    /// Returns the consumed L2 transaction hash recorded for one L1 message nonce.
    /// Missing mappings indicate that the message is not known to be consumed.
    pub fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.db.get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
    }
    /// Returns the L1 block which emitted the message for one core-contract nonce.
    /// Missing metadata is preserved as `None` for callers to handle.
    pub fn get_l1_handler_l1_block_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<u64>> {
        self.db.get_l1_handler_l1_block_by_nonce(core_contract_nonce)
    }
    /// Returns all messages sent by a given L1 transaction, as `(nonce, consumed_l2_tx_hash_if_known)`.
    pub fn get_messages_to_l2_by_l1_tx_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<Option<crate::storage::L1ToL2MessagesByL1TxHash>> {
        self.db.get_messages_to_l2_by_l1_tx_hash(l1_tx_hash)
    }
    /// Returns the status entry for a specific `(l1_tx_hash, nonce)` message index key.
    pub fn get_message_to_l2_index_entry(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<Option<crate::storage::L1ToL2MessageIndexEntry>> {
        self.db.get_message_to_l2_index_entry(l1_tx_hash, core_contract_nonce)
    }
    /// Streams saved mempool transactions from durable storage.
    /// Individual decoding failures are returned through the iterator.
    pub fn get_saved_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        self.db.get_mempool_transactions()
    }
    /// Streams at most `limit` transactions awaiting external publication.
    /// Entries retain their durable outbox identifiers for later deletion.
    pub fn get_external_outbox_transactions(
        &self,
        limit: usize,
    ) -> impl Iterator<Item = Result<ExternalOutboxEntry>> + '_ {
        self.db.get_external_outbox_transactions(limit)
    }
    /// Returns the storage backend's estimate of pending external outbox entries.
    /// The value is informational and may be approximate during concurrent writes.
    pub fn get_external_outbox_size_estimate(&self) -> Result<u64> {
        self.db.get_external_outbox_size_estimate()
    }
    /// Loads persisted devnet predeployed-account keys when configured.
    /// Production databases normally return no such fixture data.
    pub fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        self.db.get_devnet_predeployed_keys()
    }
    /// Returns the latest block whose trie update was durably applied.
    /// Recovery uses this marker to reconcile confirmed state.
    pub fn get_latest_applied_trie_update(&self) -> Result<Option<u64>> {
        self.db.get_latest_applied_trie_update()
    }
    /// Returns the latest block completed by snapshot synchronization.
    /// Absence means snapshot sync has not stored a progress marker.
    pub fn get_snap_sync_latest_block(&self) -> Result<Option<u64>> {
        self.db.get_snap_sync_latest_block()
    }
}
// Delegate these db reads/writes. These are related to specific services, and are not specific to a block view / the head projection writer handle.
impl<D: MadaraStorage> MadaraBackend<D> {
    /// Persists the L1 messaging service's synchronization cursor.
    /// Passing `None` clears the durable cursor.
    pub fn write_l1_messaging_sync_tip(&self, l1_block_n: Option<u64>) -> Result<()> {
        self.db.write_l1_messaging_sync_tip(l1_block_n)
    }
    /// Persists the external database retention cursor.
    /// Cursor ordering policy remains the responsibility of the calling service.
    pub fn write_external_db_retention_cursor(&self, block_n: u64) -> Result<()> {
        self.db.write_external_db_retention_cursor(block_n)
    }
    /// Records the L2 transaction which consumed one L1 message nonce.
    /// The mapping is delegated unchanged to durable storage.
    pub fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        self.db.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash)
    }
    /// Records the originating L1 block for one message nonce.
    /// This metadata supports status queries without an L1 lookup.
    pub fn write_l1_handler_l1_block_by_nonce(&self, core_contract_nonce: u64, l1_block_n: u64) -> Result<()> {
        self.db.write_l1_handler_l1_block_by_nonce(core_contract_nonce, l1_block_n)
    }
    /// Persists an L1-to-L2 message in the pending queue.
    /// The message remains pending until confirmation removes its nonce.
    pub fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        self.db.write_pending_message_to_l2(msg)
    }
    /// Removes one pending L1-to-L2 message by core-contract nonce.
    /// Repeated removal follows the storage backend's idempotent semantics.
    pub fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        self.db.remove_pending_message_to_l2(core_contract_nonce)
    }
    /// Stores the L1 transaction hash which emitted the L1->L2 message identified by `core_contract_nonce`.
    pub fn write_l1_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<()> {
        self.db.write_l1_txn_hash_by_nonce(core_contract_nonce, l1_tx_hash)
    }
    /// Inserts a "seen on L1" marker for the `(l1_tx_hash, nonce)` pair, if the key is missing.
    ///
    /// This is idempotent and will not overwrite an already-consumed entry.
    pub fn insert_message_to_l2_seen_marker(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<bool> {
        self.db.insert_message_to_l2_seen_marker(l1_tx_hash, core_contract_nonce)
    }
    /// Writes the consumed L2 transaction hash for the `(l1_tx_hash, nonce)` pair.
    pub fn write_message_to_l2_consumed_txn_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
        l2_tx_hash: &Felt,
    ) -> Result<()> {
        self.db.write_message_to_l2_consumed_txn_hash(l1_tx_hash, core_contract_nonce, l2_tx_hash)
    }
    /// Persists devnet predeployed-account keys for later startup reuse.
    /// This helper does not generate or transform key material.
    pub fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        self.db.write_devnet_predeployed_keys(devnet_keys)
    }
    /// Removes saved mempool rows for the supplied transaction hashes.
    /// Callers may provide any iterator without allocating an intermediate vector.
    pub fn remove_saved_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        self.db.remove_mempool_transactions(tx_hashes)
    }
    /// Persists one validated transaction in the mempool recovery store.
    /// Validation is expected to have completed before this method is called.
    pub fn write_saved_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()> {
        self.db.write_mempool_transaction(tx)
    }
    /// Appends one validated transaction to the durable external outbox.
    /// The returned identifier is used to acknowledge and delete the entry later.
    pub fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<ExternalOutboxId> {
        self.db.write_external_outbox(tx)
    }
    /// Deletes one acknowledged external outbox entry by durable identifier.
    /// No other outbox entries are affected.
    pub fn delete_external_outbox(&self, id: ExternalOutboxId) -> Result<()> {
        self.db.delete_external_outbox(id)
    }
    /// Persists the latest applied trie-update marker.
    /// Passing `None` clears the marker for an empty or rewound chain.
    pub fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()> {
        self.db.write_latest_applied_trie_update(block_n)
    }
    /// Persists snapshot synchronization progress.
    /// Passing `None` clears the stored progress marker.
    pub fn write_snap_sync_latest_block(&self, block_n: &Option<u64>) -> Result<()> {
        self.db.write_snap_sync_latest_block(block_n)
    }

    /// Revert the blockchain to a specific block hash.
    pub fn revert_to(&self, new_tip_block_hash: &Felt) -> Result<(u64, Felt)> {
        let _projection_guard = self.head_projection_write_lock.lock().expect("Poisoned head projection lock");
        let previous_tip = self.chain_head_state();
        let previous_latest_confirmed_block_n = previous_tip.confirmed_tip.ok_or_else(|| {
            anyhow::anyhow!("Cannot revert backend cache state without a confirmed block in the current chain head")
        })?;
        let previous_latest_confirmed_block_hash = self
            .db
            .get_block_info(previous_latest_confirmed_block_n)?
            .ok_or_else(|| {
                anyhow::anyhow!("Current tip block info not found for block_n={previous_latest_confirmed_block_n}")
            })?
            .block_hash;
        let requested_new_tip_block_n = self
            .db
            .find_block_hash(new_tip_block_hash)?
            .ok_or_else(|| anyhow::anyhow!("Target block hash {new_tip_block_hash:#x} not found"))?;
        let first_reverted_block_n =
            (requested_new_tip_block_n < previous_latest_confirmed_block_n).then_some(requested_new_tip_block_n + 1);
        let first_reverted_block_hash = first_reverted_block_n
            .map(|block_n| {
                self.db
                    .get_block_info(block_n)?
                    .ok_or_else(|| anyhow::anyhow!("First reverted block info not found for block_n={block_n}"))
                    .map(|info| info.block_hash)
            })
            .transpose()?;
        let (new_tip_block_n, new_tip_block_hash) = self.db.revert_to(new_tip_block_hash)?;

        let had_preconfirmed_tip =
            previous_tip.external_preconfirmed_tip.is_some() || previous_tip.internal_preconfirmed_tip.is_some();
        if requested_new_tip_block_n == previous_latest_confirmed_block_n && !had_preconfirmed_tip {
            return Ok((new_tip_block_n, new_tip_block_hash));
        }

        self.refresh_head_projection_from_db_locked().context("Refreshing head projection after revert")?;
        let refreshed_chain_tip = self.chain_head_state();
        ensure!(
            refreshed_chain_tip.confirmed_tip == Some(new_tip_block_n),
            "Refreshed chain head cache ({refreshed_chain_tip:?}) does not match reverted block_n={new_tip_block_n}",
        );

        if refreshed_chain_tip == previous_tip {
            return Ok((new_tip_block_n, new_tip_block_hash));
        }

        let stored_l1_confirmed = self.db.get_confirmed_on_l1_tip()?;
        let clamped_l1_confirmed = stored_l1_confirmed.map(|block_n| block_n.min(new_tip_block_n));
        if clamped_l1_confirmed != stored_l1_confirmed {
            self.db.write_confirmed_on_l1_tip(clamped_l1_confirmed)?;
        }
        if *self.latest_l1_confirmed.borrow() != clamped_l1_confirmed {
            self.latest_l1_confirmed.send_replace(clamped_l1_confirmed);
        }

        if let (Some(first_reverted_block_n), Some(first_reverted_block_hash)) =
            (first_reverted_block_n, first_reverted_block_hash)
        {
            let notification = ReorgNotification {
                previous_head: ReorgHead {
                    tip: previous_tip,
                    latest_confirmed_block_n: previous_latest_confirmed_block_n,
                    latest_confirmed_block_hash: previous_latest_confirmed_block_hash,
                },
                new_head: ReorgHead {
                    tip: refreshed_chain_tip,
                    latest_confirmed_block_n: new_tip_block_n,
                    latest_confirmed_block_hash: new_tip_block_hash,
                },
                first_reverted_block_n,
                first_reverted_block_hash,
            };
            let _ = self.reorg_notifications.send(notification);
        }

        Ok((new_tip_block_n, new_tip_block_hash))
    }
}
