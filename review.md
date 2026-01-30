# PR Review Notes

## Findings

1. `block_counter` and `transaction_counter` now include a `block_number` label when incremented. This makes the counters create a new time series per block (unbounded cardinality) and breaks existing queries that expect a single monotonically increasing counter. Consider keeping these counters unlabeled and using the new per-block gauges for block-specific data.

2. `apply_to_global_trie_last_seconds` records the total duration of a batch of state diffs but labels it with the last block number in the batch. When sync applies multiple blocks in one call, the per-block timing becomes inflated and misleading. If the intent is per-block timing, record per-block durations or add a batch size label.
