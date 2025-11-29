#[derive(Debug, Clone, Copy, Default)]
pub enum ColumnMemoryBudget {
    #[default]
    Other,
    Blocks,
    Contracts,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub rocksdb_name: &'static str,
    pub prefix_extractor_len: Option<usize>,
    pub budget_tier: ColumnMemoryBudget,
    pub point_lookup: bool,
}

pub const ALL_COLUMNS: &[Column] = &[
    super::blocks::BLOCK_HASH_TO_BLOCK_N_COLUMN,
    super::blocks::TX_HASH_TO_INDEX_COLUMN,
    super::blocks::BLOCK_INFO_COLUMN,
    super::blocks::BLOCK_STATE_DIFF_COLUMN,
    super::blocks::BLOCK_TRANSACTIONS_COLUMN,
    super::blocks::BLOCK_BOUNCER_WEIGHT_COLUMN,
    super::state::CONTRACT_STORAGE_COLUMN,
    super::state::CONTRACT_NONCE_COLUMN,
    super::state::CONTRACT_CLASS_HASH_COLUMN,
    super::classes::CLASS_INFO_COLUMN,
    super::classes::CLASS_COMPILED_COLUMN,
    super::classes::CLASS_MIGRATION_COLUMN,
    super::events::EVENTS_BLOOM_COLUMN,
    super::meta::META_COLUMN,
    super::meta::PRECONFIRMED_COLUMN,
    super::l1_to_l2_messages::L1_TO_L2_PENDING_MESSAGE_BY_NONCE,
    super::l1_to_l2_messages::L1_TO_L2_TXN_HASH_BY_NONCE,
    super::mempool::MEMPOOL_TRANSACTIONS_COLUMN,
    super::trie::BONSAI_CONTRACT_FLAT_COLUMN,
    super::trie::BONSAI_CONTRACT_TRIE_COLUMN,
    super::trie::BONSAI_CONTRACT_LOG_COLUMN,
    super::trie::BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
    super::trie::BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
    super::trie::BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
    super::trie::BONSAI_CLASS_FLAT_COLUMN,
    super::trie::BONSAI_CLASS_TRIE_COLUMN,
    super::trie::BONSAI_CLASS_LOG_COLUMN,
];

impl Column {
    pub const fn new(name: &'static str) -> Self {
        Self {
            rocksdb_name: name,
            prefix_extractor_len: None,
            budget_tier: ColumnMemoryBudget::Other,
            point_lookup: false,
        }
    }
    pub const fn with_prefix_extractor_len(mut self, prefix_extractor_len: usize) -> Self {
        self.prefix_extractor_len = Some(prefix_extractor_len);
        self
    }

    pub const fn use_blocks_mem_budget(mut self) -> Self {
        self.budget_tier = ColumnMemoryBudget::Blocks;
        self
    }
    pub const fn use_contracts_mem_budget(mut self) -> Self {
        self.budget_tier = ColumnMemoryBudget::Contracts;
        self
    }
    pub const fn set_point_lookup(mut self) -> Self {
        self.point_lookup = true;
        self
    }
}
