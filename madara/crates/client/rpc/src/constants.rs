use mp_chain_config::StarknetVersion;

/// Maximum number of filter keys that can be passed to the `get_events` RPC.
pub const MAX_EVENTS_KEYS: usize = 100;
/// Maximum number of events that can be fetched in a single chunk for the `get_events` RPC.
pub const MAX_EVENTS_CHUNK_SIZE: usize = 1000;
/// Blockifier does not support execution for versions earlier than that.
pub const EXECUTION_UNSUPPORTED_BELOW_VERSION: StarknetVersion = StarknetVersion::V0_13_0;
