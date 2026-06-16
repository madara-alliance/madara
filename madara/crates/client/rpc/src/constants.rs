/// Maximum number of filter keys that can be passed to the `get_events` RPC.
pub const MAX_EVENTS_KEYS: usize = 100;
/// Maximum number of events that can be fetched in a single chunk for the `get_events` RPC.
pub const MAX_EVENTS_CHUNK_SIZE: usize = 1000;
/// Maximum number of transactions accepted per `estimateFee`/`simulateTransactions` request.
/// Estimation can execute each transaction several times (L2 gas limit discovery), so the
/// per-request work must be bounded.
pub const MAX_ESTIMATE_TRANSACTIONS: usize = 100;
