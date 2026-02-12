# Orchestrator Storage Paths

This document lists the storage paths written by the orchestrator. Placeholders:
- `<agg_batch_id>`: Aggregator batch index
- `<snos_batch_id>`: SNOS batch index
- `<block_no>`: Block number
- `<blob_index>`: Blob index (1-based in aggregator batching)

## Paths (spaces between `/` to avoid UI truncation)

### Batch-scoped
- "/ artifacts / batch / <agg_batch_id> / cairo_pie.zip"
- "/ artifacts / batch / <agg_batch_id> / da_blob.json"
- "/ artifacts / batch / <agg_batch_id> / program_output.txt"
- "/ artifacts / batch / <agg_batch_id> / proof.json"  *(only if `store_audit_artifacts=true`)*
- "/ state_update / batch / <agg_batch_id>.json"
- "/ blob / batch / <agg_batch_id> / <blob_index>.txt"

### Root-level
- "/ <snos_batch_id> / cairo_pie.zip"
- "/ <snos_batch_id> / snos_output.json"
- "/ <snos_batch_id> / program_output.txt"
- "/ <block_no> / blob_data.txt"  *(L3 DA job)*
- "/ <block_no> / proof.json"  *(L3 ProofCreation download)*
- "/ <block_no> / proof_part2.json"  *(L3 ProofRegistration download)*

## Configured but not currently written
- "/ <snos_batch_id> / onchain_data.json"
- "/ artifacts / batch / <agg_batch_id> / snos_output.json"
