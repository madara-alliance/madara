//! RocksDB implementation of the durable storage write contract.
//! Individual method semantics and invariants are documented by [`MadaraStorageWrite`].

use super::*;

impl MadaraStorageWrite for RocksDBStorage {
    fn write_header(&self, header: mp_block::BlockHeaderWithSignatures) -> Result<()> {
        tracing::debug!("Writing header {}", header.header.block_number);
        let block_n = header.header.block_number;
        self.inner
            .blocks_store_block_header(header)
            .with_context(|| format!("Storing block_header for block_n={block_n}"))
    }

    fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()> {
        tracing::debug!("Writing transactions {block_n}");
        self.inner
            .blocks_store_transactions(block_n, txs)
            .with_context(|| format!("Storing transactions for block_n={block_n}"))
    }

    fn confirm_l1_messages_in_block(&self, block_n: u64) -> Result<()> {
        let Some(block_info) = self
            .inner
            .get_block_info(block_n)
            .with_context(|| format!("Reading block info before confirming L1 messages for block_n={block_n}"))?
        else {
            // Head-projection unit tests and unsafe starting-block configurations can advance a
            // logical head without locally materialized block rows. There are no transaction
            // side effects to finalize in that case.
            tracing::debug!("Skipping L1 message confirmation for block_n={block_n}: block info is not stored");
            return Ok(());
        };
        let transactions: Vec<_> =
            self.inner.get_block_transactions(block_n, 0).take(block_info.tx_hashes.len()).collect::<Result<_>>()?;
        ensure!(
            transactions.len() == block_info.tx_hashes.len(),
            "Expected {} transactions while confirming L1 messages for block_n={block_n}, found {}",
            block_info.tx_hashes.len(),
            transactions.len()
        );

        self.inner
            .messages_to_l2_write_transactions(
                transactions
                    .iter()
                    .filter_map(|value| value.transaction.as_l1_handler().zip(value.receipt.as_l1_handler())),
            )
            .with_context(|| format!("Confirming L1 messages consumed in block_n={block_n}"))
    }

    fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        tracing::debug!("Writing state diff {block_n}");

        // Update compiled_class_hash_v2 for SNIP-34 migrated classes
        if !value.migrated_compiled_classes.is_empty() {
            let migrations: Vec<(Felt, Felt)> =
                value.migrated_compiled_classes.iter().map(|m| (m.class_hash, m.compiled_class_hash)).collect();
            tracing::debug!("Updating {} class v2 hashes (SNIP-34 migrations) for block {}", migrations.len(), block_n);
            self.inner.update_class_v2_hashes(migrations).context("Updating class v2 hashes")?;
        }

        self.inner
            .blocks_store_state_diff(block_n, value)
            .with_context(|| format!("Storing state diff for block_n={block_n}"))?;
        self.inner
            .state_apply_state_diff(block_n, value)
            .with_context(|| format!("Applying state from state diff for block_n={block_n}"))
    }

    fn write_bouncer_weights(&self, block_n: u64, value: &BouncerWeights) -> Result<()> {
        tracing::debug!("Writing bouncer weights for block_n={block_n}");
        self.inner
            .blocks_store_bouncer_weights(block_n, value)
            .with_context(|| format!("Storing bouncer weights for block_n={block_n}"))
    }

    fn write_events(&self, block_n: u64, events: &[mp_receipt::EventWithTransactionHash]) -> Result<()> {
        tracing::debug!("Writing events {block_n}");
        self.inner
            .blocks_store_events_to_receipts(block_n, events)
            .with_context(|| format!("Storing events to receipts for block_n={block_n}"))?;
        self.inner
            .store_events_bloom(block_n, events)
            .with_context(|| format!("Storing events bloom filter for block_n={block_n}"))
    }

    fn write_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        tracing::debug!("Writing classes {block_n}");
        self.inner.store_classes(block_n, converted_classes)
    }

    fn update_class_v2_hashes(&self, migrations: Vec<(Felt, Felt)>) -> Result<()> {
        tracing::debug!("Updating {} class v2 hashes (SNIP-34 migrations)", migrations.len());
        self.inner.update_class_v2_hashes(migrations).context("Updating class v2 hashes")
    }

    fn replace_head_projection(&self, head_projection: &StorageHeadProjection) -> Result<()> {
        tracing::debug!("Replace head projection {head_projection:?}");
        self.inner.replace_head_projection(head_projection).context("Replacing head projection in db")
    }

    fn append_preconfirmed_content(
        &self,
        block_n: u64,
        start_tx_index: u64,
        txs: &[PreconfirmedExecutedTransaction],
    ) -> Result<()> {
        tracing::debug!(
            "Append preconfirmed content block_n={block_n}, start_tx_index={start_tx_index}, new_txs={}",
            txs.len()
        );
        self.inner
            .append_preconfirmed_content(block_n, start_tx_index, txs)
            .context("Appending to preconfirmed content to db")
    }

    fn write_preconfirmed_header(&self, header: &mp_block::header::PreconfirmedHeader) -> Result<()> {
        tracing::debug!("Write preconfirmed header block_n={}", header.block_number);
        self.inner.write_preconfirmed_header(header).context("Writing preconfirmed header")
    }

    fn delete_preconfirmed_rows_up_to(&self, confirmed_tip: u64) -> Result<()> {
        tracing::debug!("Delete preconfirmed rows up to confirmed_tip={confirmed_tip}");
        self.inner
            .delete_preconfirmed_rows_up_to(confirmed_tip)
            .context("Deleting block-scoped preconfirmed rows for confirmed GC")
    }

    fn write_confirmed_on_l1_tip(&self, block_n: Option<u64>) -> Result<()> {
        tracing::debug!("Write confirmed on l1 tip block_n={block_n:?}");
        self.inner.write_confirmed_on_l1_tip(block_n).context("Writing confirmed on l1 tip")
    }
    fn write_l1_messaging_sync_tip(&self, block_n: Option<u64>) -> Result<()> {
        tracing::debug!("Write l1 messaging tip block_n={block_n:?}");
        self.inner.write_l1_messaging_sync_tip(block_n).context("Writing l1 messaging sync tip")
    }
    fn write_external_db_retention_cursor(&self, block_n: u64) -> Result<()> {
        tracing::debug!("Write external db retention cursor block_n={block_n:?}");
        self.inner.write_external_db_retention_cursor(block_n).context("Writing external db retention cursor")
    }
    fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        tracing::debug!(
            "Write l1 handler tx hash by nonce core_contract_nonce={core_contract_nonce}, txn_hash={txn_hash:#x}"
        );
        self.inner.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash).with_context(|| {
            format!("Writing l1 handler txn hash by nonce nonce={core_contract_nonce} txn_hash={txn_hash:#x}")
        })
    }
    fn write_l1_handler_l1_block_by_nonce(&self, core_contract_nonce: u64, l1_block_n: u64) -> Result<()> {
        tracing::debug!(
            "Write l1 handler l1 block by nonce core_contract_nonce={core_contract_nonce}, l1_block_n={l1_block_n}"
        );
        self.inner.write_l1_handler_l1_block_by_nonce(core_contract_nonce, l1_block_n).with_context(|| {
            format!("Writing l1 handler l1 block by nonce nonce={core_contract_nonce} l1_block_n={l1_block_n}")
        })
    }
    fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        tracing::debug!("Write pending message to l2 nonce={}", msg.tx.nonce);
        let nonce = msg.tx.nonce;
        self.inner
            .write_pending_message_to_l2(msg)
            .with_context(|| format!("Writing pending message to l2 nonce={nonce}"))
    }
    fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        tracing::debug!("Remove pending message to l2 nonce={core_contract_nonce}");
        self.inner
            .remove_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Removing pending message to l2 nonce={core_contract_nonce}"))
    }
    fn write_l1_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<()> {
        tracing::debug!(
            "Write l1 txn hash by nonce core_contract_nonce={core_contract_nonce} l1_tx_hash_bytes={:?}",
            l1_tx_hash.0
        );
        self.inner.write_l1_txn_hash_by_nonce(core_contract_nonce, l1_tx_hash).with_context(|| {
            format!(
                "Writing l1 txn hash by nonce core_contract_nonce={core_contract_nonce} l1_tx_hash_bytes={:?}",
                l1_tx_hash.0
            )
        })
    }

    fn insert_message_to_l2_seen_marker(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<bool> {
        tracing::debug!(
            "Insert l1->l2 message seen marker l1_tx_hash_bytes={:?} nonce={core_contract_nonce}",
            l1_tx_hash.0
        );
        self.inner.insert_message_to_l2_seen_marker(l1_tx_hash, core_contract_nonce).with_context(|| {
            format!(
                "Inserting l1->l2 message seen marker l1_tx_hash_bytes={:?} nonce={core_contract_nonce}",
                l1_tx_hash.0
            )
        })
    }
    fn write_message_to_l2_consumed_txn_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
        l2_tx_hash: &Felt,
    ) -> Result<()> {
        tracing::debug!(
            "Write consumed l1->l2 message l1_tx_hash_bytes={:?} nonce={core_contract_nonce} l2_tx_hash={l2_tx_hash:#x}",
            l1_tx_hash.0
        );
        self.inner.write_message_to_l2_consumed_txn_hash(l1_tx_hash, core_contract_nonce, l2_tx_hash).with_context(
            || {
                format!(
                    "Writing consumed l1->l2 message l1_tx_hash_bytes={:?} nonce={core_contract_nonce} l2_tx_hash={l2_tx_hash:#x}",
                    l1_tx_hash.0
                )
            },
        )
    }
    fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        tracing::debug!("Write devnet keys");
        self.inner.write_devnet_predeployed_keys(devnet_keys).context("Writing devnet predeployed keys to db")
    }
    fn write_chain_info(&self, info: &StoredChainInfo) -> Result<()> {
        tracing::debug!("Write chain info");
        self.inner.write_chain_info(info)
    }
    fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()> {
        tracing::debug!("Write latest applied trie update block_n={block_n:?}");
        self.inner.write_latest_applied_trie_update(block_n).context("Writing latest applied trie update block_n")
    }
    fn write_runtime_exec_config(&self, config: &mp_chain_config::RuntimeExecutionConfig) -> Result<()> {
        tracing::debug!("Writing runtime execution config");
        self.inner.write_runtime_exec_config(config).context("Writing runtime execution config")
    }
    fn clear_runtime_exec_config(&self) -> Result<()> {
        tracing::debug!("Clearing runtime execution config");
        self.inner.clear_runtime_exec_config().context("Clearing runtime execution config")
    }
    fn write_snap_sync_latest_block(&self, block_n: &Option<u64>) -> Result<()> {
        tracing::debug!("Write snap sync latest block block_n={block_n:?}");
        self.inner.write_snap_sync_latest_block(block_n).context("Writing snap sync latest block")
    }

    fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        tracing::debug!("Remove mempool transactions");
        self.inner.remove_mempool_transactions(tx_hashes).context("Removing mempool transactions from db")
    }
    fn write_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()> {
        let tx_hash = tx.hash;
        tracing::debug!("Writing mempool transaction from db for tx_hash={tx_hash:#x}");
        self.inner
            .write_mempool_transaction(tx)
            .with_context(|| format!("Writing mempool transaction from db for tx_hash={tx_hash:#x}"))
    }
    fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<external_outbox::ExternalOutboxId> {
        let tx_hash = tx.hash;
        tracing::debug!("Writing external outbox transaction for tx_hash={tx_hash:#x}");
        self.inner
            .write_external_outbox(tx)
            .with_context(|| format!("Writing external outbox transaction for tx_hash={tx_hash:#x}"))
    }
    fn delete_external_outbox(&self, id: external_outbox::ExternalOutboxId) -> Result<()> {
        tracing::debug!("Removing external outbox transaction arrived_at_ms={} uuid={:x?}", id.arrived_at_ms, id.uuid);
        self.inner
            .delete_external_outbox(id)
            .with_context(|| format!("Deleting external outbox transaction arrived_at_ms={}", id.arrived_at_ms))
    }

    fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
        protocol_version: StarknetVersion,
    ) -> Result<(Felt, MerklizationTimings)> {
        tracing::debug!("Applying state diff to global trie start_block_n={start_block_n}");
        apply_to_global_trie(self, start_block_n, state_diffs, protocol_version)
            .context("Applying state diff to global trie")
    }

    fn compute_global_trie_staged(
        &self,
        state_diff: &StateDiff,
        protocol_version: StarknetVersion,
        block_number: u64,
    ) -> Result<(Felt, global_trie::StagedGlobalTries)> {
        tracing::debug!("Computing staged global trie for block_n={block_number}");
        compute_global_trie_staged(self, state_diff, protocol_version, block_number)
            .context("Computing staged global trie")
    }

    fn flush(&self) -> Result<()> {
        tracing::debug!("Flushing");
        self.inner.flush().context("Flushing RocksDB database")?;
        self.backup.backup_if_enabled(&self.inner).context("Backing up RocksDB database")
    }

    fn on_new_confirmed_head(&self, block_n: u64) -> Result<()> {
        tracing::debug!("on_new_confirmed_head block_n={block_n}");
        let started_at = Instant::now();
        self.snapshots.set_new_head(block_n);
        crate::warn_if_confirmed_head_phase_slow(block_n, "snapshot_head_rotation", started_at.elapsed());

        let started_at = Instant::now();
        if self.has_parallel_merkle_checkpoint(block_n)? {
            self.snapshots.pin_head(block_n);
        }
        crate::warn_if_confirmed_head_phase_slow(block_n, "checkpoint_snapshot_pin", started_at.elapsed());

        let started_at = Instant::now();
        self.metrics.update(self);
        crate::warn_if_confirmed_head_phase_slow(block_n, "db_metrics_update", started_at.elapsed());
        Ok(())
    }

    fn reconcile_confirmed_parallel_merkle_state(&self, block_n: Option<u64>, context: &str) -> Result<()> {
        RocksDBStorage::reconcile_confirmed_parallel_merkle_state(self, block_n, context)
    }

    fn remove_all_blocks_starting_from(&self, starting_from_block_n: u64) -> Result<()> {
        tracing::debug!("remove_all_blocks_starting_from starting_from_block_n={starting_from_block_n}");
        self.inner
            .remove_all_blocks_starting_from(starting_from_block_n)
            .with_context(|| format!("Removing all blocks in range [{starting_from_block_n}..] from database"))
    }

    fn get_state_root_hash(&self) -> Result<Felt> {
        get_state_root(self, StarknetVersion::LATEST)
    }

    fn get_state_root_hash_at_version(&self, protocol_version: StarknetVersion) -> Result<Felt> {
        get_state_root(self, protocol_version)
    }

    /// Reverts the blockchain state to a specific block hash during a chain reorganization.
    ///
    /// This function performs a complete rollback of the blockchain state to a target block,
    /// which is typically the common ancestor between the current chain and a new canonical chain.
    /// It ensures data consistency by reverting all state components including Bonsai tries,
    /// block data, contract state, and class definitions.
    ///
    /// # Arguments
    ///
    /// * `new_tip_block_hash` - The block hash to revert to. This must be an existing block
    ///   that is an ancestor of the current head projection. The block with this hash will become
    ///   the new head projection after the revert completes.
    ///
    /// # Returns
    ///
    /// Returns `Ok((block_number, block_hash))` where:
    /// * `block_number` - The block number of the new head projection
    /// * `block_hash` - The block hash of the new head projection (same as input `new_tip_block_hash`)
    ///
    /// # Implementation Details
    ///
    /// The revert process performs the following steps in order:
    ///
    /// 1. **Validation**: Finds and validates the target block exists and is finalized
    /// 2. **Range Calculation**: Determines the range of blocks to remove (target_block + 1..=current_tip)
    /// 3. **Bonsai Tries Revert**: Reverts the contract, contract_storage, and class tries to the target block's state
    /// 4. **Trie Commit**: Commits the reverted tries to ensure consistency
    /// 5. **Root Verification**: Refuses to publish a target whose materialized root does not match
    /// 6. **Atomic Head Commit**: Publishes the target with preconfirmed/L1/trie recovery metadata
    /// 7. **Suffix Cleanup**: Removes future blocks and their versioned state in reverse order
    /// 8. **Snapshot Update**: Updates the in-memory snapshot inventory to the target block
    /// 9. **Database Flush**: Ensures all changes are persisted to disk
    ///
    /// # Notes
    ///
    /// * L1-message preflight runs before destructive writes. If reverted L1-handler nonces
    ///   are missing source-block mappings, this function fails early without mutating chain state.
    /// * After calling this function, the caller MUST refresh the backend's head projection
    ///   by reading from the database, as this function only updates the database state.
    /// * This function does not stop services or shutdown the process. Lifecycle side-effects
    ///   are managed by upper layers (for example admin RPC orchestration).
    /// * This is a destructive operation - all blocks after the target block are permanently removed.
    /// * The head commit is the reorg's linearization point. A crash before it recovers the old
    ///   confirmed head; a crash after it resumes reverse-order suffix cleanup from the new head.
    fn revert_to(&self, new_tip_block_hash: &Felt) -> Result<(u64, Felt)> {
        super::reorg::execute_reorg(self, new_tip_block_hash)
    }
}
