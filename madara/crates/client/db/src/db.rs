use crate::MadaraStorageError;
use mp_block::{EventWithInfo, MadaraBlockInfo, TransactionWithReceipt};
use mp_class::{ClassInfo, CompiledSierra};
use mp_convert::Felt;
use mp_receipt::Event;
use mp_state_update::StateDiff;

#[derive(Debug, Clone, Copy)]
pub struct EventFilter {
    pub start_block: u64,
    pub start_event_index: usize,
    pub end_block: u64,
    pub from_address: Option<&Felt>,
    pub keys_pattern: Option<&[Vec<Felt>]>,
    pub max_events: usize,
}

impl EventFilter {
    pub fn matches(&self, event: Event) -> bool {
        self.from_address.is_none_or(|from_address| from_address == &event.from_address) &&
        // Check if the number of keys in the event matches the number of provided key patterns.
        self.keys_pattern.is_none_or(|keys_pattern| {
            keys_pattern.len() <= event.keys.len() &&
            keys_pattern.iter().zip(event.keys.iter()).all(|(pattern, key)| {
                pattern.is_empty() || pattern.contains(key)
            })
        });
    }
}


#[derive(Debug, Clone, Copy)]
pub struct GetEventContinuation {
    pub block: u64,
    pub transaction_index: u64,
    pub event_index: u64,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct TxIndex {
    pub block_n: u64,
    pub tx_index: u64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ClassInfoWithBlockN {
    pub block_n: u64,
    pub class_info: ClassInfo,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct CompiledSierraWithBlockN {
    pub block_n: u64,
    pub compiled_sierra: CompiledSierra,
}

pub trait DBBackend {
    fn get_storage_at(
        &self,
        block_n: u64,
        contract_address: &Felt,
        key: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError>;
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>, MadaraStorageError>;
    fn get_contract_class_hash_at(
        &self,
        block_n: u64,
        contract_address: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError>;
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool, MadaraStorageError>;

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>, MadaraStorageError>;
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<TxIndex>, MadaraStorageError>;
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>, MadaraStorageError>;
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>, MadaraStorageError>;
    fn get_transaction(
        &self,
        block_n: u64,
        tx_index: u64,
    ) -> Result<Option<TransactionWithReceipt>, MadaraStorageError>;
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt, MadaraStorageError>> + '_;

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>, MadaraStorageError>;
    fn get_class_compiled(
        &self,
        compiled_class_hash: &Felt,
    ) -> Result<Option<CompiledSierraWithBlockN>, MadaraStorageError>;

    fn get_events(&self, event_filter: EventFilter) -> Result<Vec<EventWithInfo>, MadaraStorageError>;
}
