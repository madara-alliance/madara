# Close Block Observability Plan (Metrics + Logs)

## Goal

Provide a scalable observability model for the **admin close_block flow** that:

- Keeps **Prometheus** low‑cardinality for trends and alerts.
- Captures **per‑block, per‑function timings** in **logs (Loki)** for drill‑down.
- Uses the existing Loki log capture path (no OTLP logs export changes).

## Scope

Covers the end‑to‑end `close_block` flow in `madara/crates/client/block_production/src/lib.rs` and its downstream DB/merklization work.

---

## High‑Level Architecture

- **Metrics** (Prometheus): aggregated histograms (no `block_number`).
- **Logs** (Loki): one structured event per block with full per‑function timings.

---

## Work Breakdown (Small Chunks)

### Chunk 0 — Instrumentation: Per‑block Structured Log Event

Add one structured log event emitted at the end of `close_block` capturing all per‑block fields.

**Log event name**
Use a single constant name (e.g., `event="close_block_complete"`).

**Proposed fields (log attributes)**

- `block_number`
- `tx_count`
- `event_count`
- `close_block_total_ms`
- `close_preconfirmed_ms`
- `block_production_ms`
- `block_close_ms`
- `state_diff_len`
- `declared_classes`
- `deployed_contracts`
- `storage_diffs`
- `nonce_updates`
- `consumed_l1_nonces`
- `bouncer_l1_gas`
- `bouncer_sierra_gas`
- `bouncer_n_events`
- `bouncer_message_segment_length`
- `bouncer_state_diff_size`
- `merklization_ms`
- `contract_trie_ms`
- `class_trie_ms`
- `get_full_block_ms`
- `commitments_ms`
- `block_hash_ms`
- `db_write_ms`

**Implementation detail**

- Use `tracing::info!` with structured fields (existing formatter + best practices).
- All per‑block values come from existing measurements and metrics already captured in the diff.

**Acceptance**

- One log event per block produced.
- All fields present and queryable in Loki.

---

### Chunk 1 — Metrics Cleanup (Remove `block_number` label)

Convert high‑cardinality per‑block metrics into **logs only**, leaving low‑cardinality histograms in Prometheus.

**Remove `block_number` label from**

- `block_counter`
- `transaction_counter`
- `block_gauge`
- `block_production_time_last`
- `block_close_time_last`
- `close_preconfirmed_last`
- `close_block_total_last`
- `block_declared_classes_count`
- `block_deployed_contracts_count`
- `block_storage_diffs_count`
- `block_nonce_updates_count`
- `block_state_diff_length`
- `block_event_count`
- `block_bouncer_l1_gas`
- `block_bouncer_sierra_gas`
- `block_bouncer_n_events`
- `block_bouncer_message_segment_length`
- `block_bouncer_state_diff_size`
- `block_consumed_l1_nonces_count`
- DB per‑block `*_last` metrics:
  - `get_full_block_with_classes_last`
  - `block_commitments_compute_last`
  - `apply_to_global_trie_last`
  - `block_hash_compute_last`
  - `db_write_block_parts_last`
  - `contract_trie_root_last`
  - `class_trie_root_last`

**Keep histograms** (Prometheus)

- `close_block_total_duration`
- `close_preconfirmed_duration`
- `block_production_time`
- `block_close_time`
- `apply_to_global_trie_duration`
- `contract_trie_root_duration`
- `class_trie_root_duration`
- `get_full_block_with_classes_duration`
- `block_commitments_compute_duration`
- `block_hash_compute_duration`
- `db_write_block_parts_duration`

---

### Chunk 2 — Grafana Dashboard Updates (Madara Overview)

Replace per‑block time‑series panels with:

1. **Trends panels (Prometheus histograms)**

- Show p50/p95 for:
  - close block total
  - apply_to_global_trie
  - block hash
  - db write

2. **Loki table panel**

- Query example:
  - `{service="madara"} |= "close_block_complete"`
- Columns:
  - `block_number`, `close_block_total_ms`, `merklization_ms`, `block_hash_ms`, etc.

---

---

## Mapping Table: Destination (Prometheus vs Loki)

### Prometheus (aggregate only)

- `close_block_total_duration`
- `close_preconfirmed_duration`
- `block_production_time`
- `block_close_time`
- `apply_to_global_trie_duration`
- `contract_trie_root_duration`
- `class_trie_root_duration`
- `get_full_block_with_classes_duration`
- `block_commitments_compute_duration`
- `block_hash_compute_duration`
- `db_write_block_parts_duration`

### Loki (per‑block detail)

All fields previously tied to `block_number` label (see Chunk 1), plus any additional per‑block values you need.

---

## Execution Order (Suggested)

1. Chunk 0 (add log event)
2. Chunk 1 (remove block_number label from metrics)
3. Chunk 2 (Grafana updates: madara-overview)

---

## Approval Checklist

- [ ] Confirm the log event field list.
- [ ] Confirm which metrics should remain as Prometheus histograms.
- [ ] Confirm Grafana panel changes (trend + Loki table).
