#[derive(Debug, Clone, Copy, Default)]
pub enum ColumnMemoryBudget {
    #[default]
    Other,
    Blocks,
    Contracts,
}

pub struct Column {
    pub rocksdb_name: &'static str,
    pub prefix_extractor_len: Option<usize>,
    pub budget_tier: ColumnMemoryBudget,
    pub point_lookup: bool,
}

impl Column {
    pub const fn new(name: &'static str) -> Self {
        Self { rocksdb_name: name, prefix_extractor_len: None, budget_tier: ColumnMemoryBudget::Other, point_lookup: false }
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

pub const ALL_COLUMNS: &[&Column] = &[
    super::state::CONTRACT_STORAGE_COLUMN,
    super::state::CONTRACT_NONCE_COLUMN,
    super::state::CONTRACT_CLASS_HASH_COLUMN,
    super::blocks::BLOCK_HASH_TO_BLOCK_N_COLUMN,
    super::blocks::TX_HASH_TO_INDEX_COLUMN,
    super::blocks::BLOCK_INFO_COLUMN,
    super::blocks::BLOCK_STATE_DIFF_COLUMN,
    super::blocks::BLOCK_TRANSACTIONS_COLUMN,
];
