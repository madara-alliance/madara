pub mod lib;
pub mod subscribe_events;
pub mod subscribe_new_heads;
pub mod subscribe_pending_transactions;
pub mod subscribe_transaction_status;

const BLOCK_PAST_LIMIT: u64 = 1024;
const ADDRESS_FILTER_LIMIT: u64 = 128;
