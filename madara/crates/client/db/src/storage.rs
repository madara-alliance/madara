use crate::preconfirmed::PreconfirmedExecutedTransaction;
use crate::prelude::*;
use mp_block::{
    header::PreconfirmedHeader, BlockHeaderWithSignatures, EventWithInfo, MadaraBlockInfo, TransactionWithReceipt,
};
use mp_class::{ClassInfo, CompiledSierra, ConvertedClass};
use mp_receipt::{Event, EventWithTransactionHash};
use mp_state_update::StateDiff;
use mp_transactions::{validated::ValidatedTransaction, L1HandlerTransactionWithFee};
use starknet_api::core::ChainId;

#[derive(Debug, Clone)]
pub struct EventFilter {
    pub start_block: u64,
    pub start_event_index: usize,
    pub end_block: u64,
    pub from_address: Option<Felt>,
    pub keys_pattern: Option<Vec<Vec<Felt>>>,
    pub max_events: usize,
}

impl EventFilter {
    pub fn matches(&self, event: &Event) -> bool {
        self.from_address.as_ref().is_none_or(|from_address| from_address == &event.from_address) &&
        // Check if the number of keys in the event matches the number of provided key patterns.
        self.keys_pattern.as_ref().is_none_or(|keys_pattern| {
            keys_pattern.len() <= event.keys.len() &&
            keys_pattern.iter().zip(event.keys.iter()).all(|(pattern, key)| {
                pattern.is_empty() || pattern.contains(key)
            })
        })
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct StorageTxIndex {
    pub block_number: u64,
    pub transaction_index: u64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ClassInfoWithBlockN {
    pub block_number: u64,
    pub class_info: ClassInfo,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct CompiledSierraWithBlockN {
    pub block_number: u64,
    pub compiled_sierra: Arc<CompiledSierra>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DevnetPredeployedContractAccount {
    pub address: Felt,
    pub secret: Felt,
    pub pubkey: Felt,
    pub class_hash: Felt,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DevnetPredeployedKeys(pub Vec<DevnetPredeployedContractAccount>);

#[derive(Clone, Debug)]
pub enum StorageChainTip {
    /// Empty pre-genesis state.
    Empty,
    /// Latest block is closed.
    Confirmed(u64),
    /// Latest block is a preconfirmed block.
    Preconfirmed { header: PreconfirmedHeader, content: Vec<PreconfirmedExecutedTransaction> },
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct StoredChainInfo {
    pub chain_id: ChainId,
    pub chain_name: String,
}

/// Trait abstracting over the storage interface.
pub trait MadaraStorageRead: Send + Sync + 'static {
    // Blocks

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>>;
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<StorageTxIndex>>;
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>>;
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>>;
    fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>>;
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_;

    // State

    fn get_storage_at(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>>;
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>>;
    fn get_contract_class_hash_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>>;
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool>;

    // Classes

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>>;
    fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<CompiledSierraWithBlockN>>;

    // Events

    fn get_events(&self, event_filter: EventFilter) -> Result<Vec<EventWithInfo>>;

    // Meta

    fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>>;
    fn get_chain_tip(&self) -> Result<StorageChainTip>;
    fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>>;
    fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>>;
    fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>>;
    fn get_latest_applied_trie_update(&self) -> Result<Option<u64>>;

    // L1 to L2 messages

    fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>>;
    fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>>;
    fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>>;

    // Mempool

    fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_;
}

/// Trait abstracting over the storage interface.
pub trait MadaraStorageWrite: Send + Sync + 'static {
    fn write_header(&self, header: BlockHeaderWithSignatures) -> Result<()>;
    fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()>;
    fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()>;
    fn write_events(&self, block_n: u64, txs: &[EventWithTransactionHash]) -> Result<()>;
    fn write_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()>;

    fn replace_chain_tip(&self, chain_tip: &StorageChainTip) -> Result<()>;
    fn append_preconfirmed_content(&self, start_tx_index: u64, txs: &[PreconfirmedExecutedTransaction]) -> Result<()>;
    /// Set the latest block confirmed on l1.
    fn write_confirmed_on_l1_tip(&self, block_n: Option<u64>) -> Result<()>;
    /// Write the latest l1_block synced for the messaging worker.
    fn write_l1_messaging_sync_tip(&self, l1_block_n: Option<u64>) -> Result<()>;

    fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()>;
    fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()>;
    fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()>;

    fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()>;
    fn write_chain_info(&self, info: &StoredChainInfo) -> Result<()>;
    fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()>;

    fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()>;
    fn write_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()>;

    /// Write a state diff to the global tries.
    /// Returns the new state root.
    fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
    ) -> Result<Felt>;

    fn flush(&self) -> Result<()>;

    /// Called everytime a new block_n is fully saved and marked as confirmed.
    fn on_new_confirmed_head(&self, block_n: u64) -> Result<()>;
}

/// Trait alias for `MadaraStorageRead + MadaraStorageWrite`.
pub trait MadaraStorage: MadaraStorageRead + MadaraStorageWrite {}
impl<T: MadaraStorageRead + MadaraStorageWrite> MadaraStorage for T {}
