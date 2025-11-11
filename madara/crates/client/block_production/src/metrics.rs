use mc_analytics::{register_counter_metric_instrument, register_gauge_metric_instrument};
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, InstrumentationScope, KeyValue};

#[derive(Debug)]
pub struct BlockProductionMetrics {
    pub block_gauge: Gauge<u64>,
    pub block_counter: Counter<u64>,
    pub transaction_counter: Counter<u64>,
    // Batcher metrics
    pub batch_to_execute_size: Gauge<u64>,
    // Executor state metrics
    pub executor_declared_classes_size: Gauge<u64>,
    pub executor_consumed_nonces_size: Gauge<u64>,
    pub executor_deployed_contracts_size: Gauge<u64>,
    pub executor_consumed_core_contract_nonces_size: Gauge<u64>,
    // Layered state adapter cache metrics
    pub layered_cache_blocks: Gauge<u64>,
    pub layered_cache_size: Gauge<u64>,
    pub layered_cache_state_maps_size: Gauge<u64>,
    pub layered_cache_classes_size: Gauge<u64>,
    pub layered_cache_l1_to_l2_messages_size: Gauge<u64>,
    // Channel metrics
    pub executor_batch_channel_pending: Gauge<u64>,
    pub executor_commands_channel_pending: Gauge<u64>,
    pub bypass_tx_channel_pending: Gauge<u64>,
}

impl BlockProductionMetrics {
    pub fn register() -> Self {
        // Register meter
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.block_production.opentelemetry")
                .with_attributes([KeyValue::new("crate", "block_production")])
                .build(),
        );

        let block_gauge = register_gauge_metric_instrument(
            &meter,
            "block_produced_no".to_string(),
            "A gauge to show block state at given time".to_string(),
            "block".to_string(),
        );
        let block_counter = register_counter_metric_instrument(
            &meter,
            "block_produced_count".to_string(),
            "A counter to show block state at given time".to_string(),
            "block".to_string(),
        );
        let transaction_counter = register_counter_metric_instrument(
            &meter,
            "transaction_counter".to_string(),
            "A counter to show transaction state for the given block".to_string(),
            "transaction".to_string(),
        );

        let batch_to_execute_size = register_gauge_metric_instrument(
            &meter,
            "batch_to_execute_size".to_string(),
            "Size of batches being sent to executor".to_string(),
            "transaction".to_string(),
        );

        let executor_declared_classes_size = register_gauge_metric_instrument(
            &meter,
            "executor_declared_classes_size".to_string(),
            "Number of declared classes in executor state".to_string(),
            "class".to_string(),
        );

        let executor_consumed_nonces_size = register_gauge_metric_instrument(
            &meter,
            "executor_consumed_nonces_size".to_string(),
            "Number of consumed L1 to L2 message nonces in executor state".to_string(),
            "nonce".to_string(),
        );

        let executor_deployed_contracts_size = register_gauge_metric_instrument(
            &meter,
            "executor_deployed_contracts_size".to_string(),
            "Number of deployed contracts tracked in CurrentBlockState".to_string(),
            "contract".to_string(),
        );

        let executor_consumed_core_contract_nonces_size = register_gauge_metric_instrument(
            &meter,
            "executor_consumed_core_contract_nonces_size".to_string(),
            "Number of consumed core contract nonces in CurrentBlockState".to_string(),
            "nonce".to_string(),
        );

        let layered_cache_blocks = register_gauge_metric_instrument(
            &meter,
            "layered_cache_blocks".to_string(),
            "Number of cached blocks in the layered state adapter".to_string(),
            "block".to_string(),
        );

        let layered_cache_size = register_gauge_metric_instrument(
            &meter,
            "layered_cache_size".to_string(),
            "Approximate size of the layered state adapter cache in bytes".to_string(),
            "byte".to_string(),
        );

        let layered_cache_state_maps_size = register_gauge_metric_instrument(
            &meter,
            "layered_cache_state_maps_size".to_string(),
            "Total number of state map entries across all cached blocks".to_string(),
            "entry".to_string(),
        );

        let layered_cache_classes_size = register_gauge_metric_instrument(
            &meter,
            "layered_cache_classes_size".to_string(),
            "Total number of classes across all cached blocks".to_string(),
            "class".to_string(),
        );

        let layered_cache_l1_to_l2_messages_size = register_gauge_metric_instrument(
            &meter,
            "layered_cache_l1_to_l2_messages_size".to_string(),
            "Total number of L1 to L2 messages across all cached blocks".to_string(),
            "message".to_string(),
        );

        let executor_batch_channel_pending = register_gauge_metric_instrument(
            &meter,
            "executor_batch_channel_pending".to_string(),
            "Number of pending batches in the executor batch channel".to_string(),
            "batch".to_string(),
        );

        let executor_commands_channel_pending = register_gauge_metric_instrument(
            &meter,
            "executor_commands_channel_pending".to_string(),
            "Number of pending commands in the executor commands channel".to_string(),
            "command".to_string(),
        );

        let bypass_tx_channel_pending = register_gauge_metric_instrument(
            &meter,
            "bypass_tx_channel_pending".to_string(),
            "Number of pending transactions in the bypass channel".to_string(),
            "transaction".to_string(),
        );

        Self {
            block_gauge,
            block_counter,
            transaction_counter,
            batch_to_execute_size,
            executor_declared_classes_size,
            executor_consumed_nonces_size,
            executor_deployed_contracts_size,
            executor_consumed_core_contract_nonces_size,
            layered_cache_blocks,
            layered_cache_size,
            layered_cache_state_maps_size,
            layered_cache_classes_size,
            layered_cache_l1_to_l2_messages_size,
            executor_batch_channel_pending,
            executor_commands_channel_pending,
            bypass_tx_channel_pending,
        }
    }
}
