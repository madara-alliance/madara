//! RocksDB implementation of the block and service storage read contract.
//! Individual method semantics are documented by [`MadaraStorageRead`].

use super::*;

impl MadaraStorageRead for RocksDBStorage {
    // Blocks

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        self.inner
            .find_block_hash(block_hash)
            .with_context(|| format!("Finding block number for block_hash={block_hash:#x}"))
    }
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<StorageTxIndex>> {
        self.inner
            .find_transaction_hash(tx_hash)
            .with_context(|| format!("Finding transaction index for tx_hash={tx_hash:#x}"))
    }
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        self.inner.get_block_info(block_n).with_context(|| format!("Getting block info for block_n={block_n}"))
    }
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>> {
        self.inner
            .get_block_state_diff(block_n)
            .with_context(|| format!("Getting block state diff for block_n={block_n}"))
    }

    fn get_block_bouncer_weights(&self, block_n: u64) -> Result<Option<BouncerWeights>> {
        self.inner
            .get_block_bouncer_weight(block_n)
            .with_context(|| format!("Getting block bouncer weights for block_n={block_n}"))
    }
    fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        self.inner
            .get_transaction(block_n, tx_index)
            .with_context(|| format!("Getting block transaction for block_n={block_n} tx_index={tx_index}"))
    }
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_ {
        self.inner.get_block_transactions(block_n, from_tx_index).map(move |e| {
            e.with_context(|| format!("Getting block transactions for block_n={block_n} from_tx_index={from_tx_index}"))
        })
    }

    // State

    fn get_storage_at(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        self.inner.get_storage_at(block_n, contract_address, key).with_context(|| {
            format!("Getting storage value for block_n={block_n} contract_address={contract_address:#x} key={key:#x}")
        })
    }
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner
            .get_contract_nonce_at(block_n, contract_address)
            .with_context(|| format!("Getting nonce for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn get_contract_class_hash_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner
            .get_contract_class_hash_at(block_n, contract_address)
            .with_context(|| format!("Getting class_hash for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool> {
        self.inner.is_contract_deployed_at(block_n, contract_address).with_context(|| {
            format!("Checking if contract is deployed for block_n={block_n} contract_address={contract_address:#x}")
        })
    }

    // Classes

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>> {
        self.inner.get_class(class_hash).with_context(|| format!("Getting class info for class_hash={class_hash:#x}"))
    }
    fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<CompiledSierraWithBlockN>> {
        self.inner
            .get_class_compiled(compiled_class_hash)
            .with_context(|| format!("Getting class compiled for compiled_class_hash={compiled_class_hash:#x}"))
    }

    // Events

    fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        self.inner.get_filtered_events(filter.clone()).with_context(|| format!("Getting events for filter={filter:?}"))
    }

    // Meta

    fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        self.inner.get_devnet_predeployed_keys().context("Getting devnet predeployed contracts keys")
    }
    fn get_head_projection(&self) -> Result<StorageHeadProjection> {
        self.inner.get_head_projection().context("Getting head projection from db")
    }
    fn get_preconfirmed_block_data(
        &self,
        block_n: u64,
    ) -> Result<Option<(mp_block::header::PreconfirmedHeader, Vec<PreconfirmedExecutedTransaction>)>> {
        self.inner
            .get_preconfirmed_block_data(block_n)
            .with_context(|| format!("Getting preconfirmed block data for block_n={block_n}"))
    }
    fn get_latest_preconfirmed_header_block_n(&self) -> Result<Option<u64>> {
        self.inner.get_latest_preconfirmed_header_block_n().context("Getting latest preconfirmed header block number")
    }
    fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>> {
        self.inner.get_confirmed_on_l1_tip().context("Getting confirmed block on l1 tip")
    }
    fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        self.inner.get_l1_messaging_sync_tip().context("Getting l1 messaging sync tip")
    }
    fn get_external_db_retention_cursor(&self) -> Result<Option<u64>> {
        self.inner.get_external_db_retention_cursor().context("Getting external db retention cursor")
    }
    fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>> {
        self.inner.get_stored_chain_info().context("Getting stored chain info from db")
    }
    fn get_latest_applied_trie_update(&self) -> Result<Option<u64>> {
        self.inner.get_latest_applied_trie_update().context("Getting latest applied trie update info from db")
    }
    fn get_runtime_exec_config(
        &self,
        backend_chain_config: &mp_chain_config::ChainConfig,
    ) -> Result<Option<mp_chain_config::RuntimeExecutionConfig>> {
        self.inner.get_runtime_exec_config(backend_chain_config).context("Getting runtime execution config from db")
    }
    fn get_snap_sync_latest_block(&self) -> Result<Option<u64>> {
        self.inner.get_snap_sync_latest_block().context("Getting snap sync latest block from db")
    }

    // L1 to L2 messages

    fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.inner
            .get_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Getting pending message to l2 with nonce={core_contract_nonce}"))
    }
    fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.inner
            .get_next_pending_message_to_l2(start_nonce)
            .with_context(|| format!("Getting next pending message to l2 with start_nonce={start_nonce}"))
    }
    fn get_l1_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<mp_convert::L1TransactionHash>> {
        self.inner
            .get_l1_txn_hash_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting l1 txn hash by nonce={core_contract_nonce}"))
    }
    fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.inner
            .get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting next pending message to l2 with nonce={core_contract_nonce}"))
    }
    fn get_l1_handler_l1_block_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<u64>> {
        self.inner
            .get_l1_handler_l1_block_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting l1 handler l1 block by nonce={core_contract_nonce}"))
    }
    fn get_messages_to_l2_by_l1_tx_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<Option<crate::storage::L1ToL2MessagesByL1TxHash>> {
        self.inner
            .get_messages_to_l2_by_l1_tx_hash(l1_tx_hash)
            .with_context(|| format!("Getting messages to l2 by l1_tx_hash_bytes={:?}", l1_tx_hash.0))
    }
    fn get_message_to_l2_index_entry(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<Option<crate::storage::L1ToL2MessageIndexEntry>> {
        self.inner.get_message_to_l2_index_entry(l1_tx_hash, core_contract_nonce).with_context(|| {
            format!(
                "Getting l1->l2 message index entry l1_tx_hash_bytes={:?} nonce={core_contract_nonce}",
                l1_tx_hash.0
            )
        })
    }

    // Mempool

    fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        self.inner.get_mempool_transactions().map(|res| res.context("Getting mempool transactions"))
    }
    fn get_external_outbox_transactions(
        &self,
        limit: usize,
    ) -> impl Iterator<Item = Result<external_outbox::ExternalOutboxEntry>> + '_ {
        self.inner.iter_external_outbox(limit).map(|res| res.context("Getting external outbox transactions"))
    }

    fn get_external_outbox_size_estimate(&self) -> Result<u64> {
        self.inner.external_outbox_size_estimate().context("Getting external outbox size estimate")
    }
}
