# Orchestrator Test Strategy - Greedy Mode Only

## Current Test Status (After SQS Removal)

The orchestrator now uses **greedy mode exclusively** - jobs are polled from MongoDB atomically rather than consumed from SQS queues. Only the **WorkerTrigger** SQS queue remains for EventBridge-triggered worker execution.

---

## Test Categories & Status

### ✅ 1. Database Tests (`tests/database/mod.rs`)
**Status:** Should work as-is (no changes needed)
- Job CRUD operations
- Batch operations
- Atomic job claiming
- Query filters
- **Count:** ~49 tests

**Required Changes:** None - database operations are independent of queue mode

---

### ⚠️ 2. Job Processing Tests (`tests/jobs/mod.rs` + job-specific)
**Status:** Need updates - remove SQS queue expectations
**Current Issues:**
- Tests expect jobs to be queued to SQS (`process_queue_name()`, `verify_queue_name()`)
- Mock queue client expects `send_message()` calls
- Tests consume from job processing queues

**Files Affected:**
- `tests/jobs/mod.rs` (23 tests)
- `tests/jobs/snos_job/mod.rs` (11 tests)
- `tests/jobs/proving_job/mod.rs` (6 tests)
- `tests/jobs/aggregator_job/mod.rs` (6 tests)
- `tests/jobs/da_job/mod.rs` (6 tests)
- `tests/jobs/state_update_job/mod.rs` (8 tests)

**Required Changes:**
- Remove `queue.expect_send_message()` mocks for job queues
- Remove `consume_message_from_queue()` calls for job queues
- Test that jobs transition to correct status in DB (without SQS verification)
- Keep `available_at` timestamp checks for delayed verification

**New Tests Needed:**
1. ✨ Test atomic job claiming with multiple orchestrators
2. ✨ Test job status transitions without SQS
3. ✨ Test `available_at` field for delayed verification
4. ✨ Test orphan job recovery (claim timeout)
5. ✨ Test job creation caps (max_concurrent_created_*_jobs)

---

### ⚠️ 3. Worker Trigger Tests (`tests/workers/`)
**Status:** Major refactor needed
**Current Issues:**
- Workers tested via SQS job queue consumption
- Need to test greedy polling behavior instead

**Files Affected:**
- `tests/workers/snos/mod.rs` (4 tests)
- `tests/workers/proving/mod.rs` (2 tests)
- `tests/workers/update_state/mod.rs` (6 tests)
- `tests/workers/batching/mod.rs` (16 tests)
- `tests/workers/batching/e2e.rs` (2 tests)

**Current Test Pattern (SQS-based):**
```rust
// OLD: Test creates jobs, expects them in SQS queue
queue.expect_send_message()
    .with(eq(job_type.process_queue_name()), ...)
    .returning(|_, _, _| Ok(()));
```

**New Test Pattern (Greedy-based):**
```rust
// NEW: Test creates jobs, trigger polls and claims them
trigger.run_worker_if_enabled(config).await?;
// Verify job was claimed and processed via DB status
let job = config.database().get_job_by_id(job_id).await?;
assert_eq!(job.status, JobStatus::LockedForProcessing);
```

**New Tests Needed:**
1. ✨ Test SNOS trigger creates jobs for closed batches (with cap)
2. ✨ Test ProofCreation trigger creates jobs for SNOS without successors (with cap, with sorting)
3. ✨ Test Aggregator trigger creates jobs (with cap, with orphan healing)
4. ✨ Test StateTransition trigger for L2/L3
5. ✨ Test orphan healing for ProofCreation and Aggregator
6. ✨ Test greedy polling respects `available_at` timestamp
7. ✨ Test greedy polling priority (PendingRetry before Created)
8. ✨ Test concurrent job claiming (atomicity)
9. ✨ Test poll interval configuration

---

### ⚠️ 4. Server/API Tests (`tests/server/`)
**Status:** Need updates - remove SQS expectations
**Files Affected:**
- `tests/server/job_routes.rs` (12 tests)
- `tests/server/admin_routes.rs` (8 tests)
- `tests/server/jobs_by_status.rs` (2 tests)
- `tests/server/mod.rs` (4 tests)

**Current Issues:**
- Tests verify jobs are queued to SQS after API calls
- E.g., POST `/jobs/{id}/process` should add to SQS queue

**Required Changes:**
- Remove queue consumption verification
- Instead verify job status changes in DB
- Keep WorkerTrigger queue tests (EventBridge integration)

**New Tests Needed:**
1. ✨ Test job creation API sets correct initial status
2. ✨ Test job retry API updates status and clears claim
3. ✨ Test bulk operations work with greedy mode

---

### ⚠️ 5. Queue Tests (`tests/queue/mod.rs`)
**Status:** Mostly obsolete, keep WorkerTrigger tests only
**Files Affected:**
- `tests/queue/mod.rs` (2 tests)

**Required Changes:**
- Remove tests for job processing queues
- Keep tests for WorkerTrigger queue only
- Test WorkerTrigger message parsing and handling

**New Tests Needed:**
1. ✨ Test WorkerTrigger queue consumption
2. ✨ Test WorkerTrigger spawns correct trigger handler
3. ✨ Test EventWorker only accepts WorkerTrigger queue type

---

### ✅ 6. Setup Tests (`tests/setup/`)
**Status:** Update for WorkerTrigger only
**Files Affected:**
- `tests/setup/sqs.rs` (4 tests)
- `tests/setup/mod.rs`

**Required Changes:**
- Update SQS setup tests to only create WorkerTrigger queue
- Remove DLQ creation tests
- Remove job queue setup tests

---

### ✅ 7. Infrastructure Tests (Alerts, Compression, Storage)
**Status:** Should work as-is
**Files:**
- `tests/alerts/mod.rs` (1 test)
- `tests/compression/mod.rs` (8 tests)
- `tests/data_storage/mod.rs` (2 tests)

**Required Changes:** None

---

## New Test Suites to Create

### 1. Greedy Worker Tests (`tests/workers/greedy/`)
**Purpose:** Test greedy polling, atomic claiming, and concurrency

```rust
// Test Structure
tests/workers/greedy/
├── claiming.rs          // Atomic job claiming tests
├── polling.rs           // Poll interval, available_at tests
├── concurrency.rs       // Multi-orchestrator tests
├── orphan_recovery.rs   // Timeout and healing tests
└── mod.rs
```

**Tests:**
1. ✨ `test_atomic_claim_single_orchestrator` - One orchestrator claims jobs
2. ✨ `test_atomic_claim_multiple_orchestrators` - Two orchestrators compete for same job
3. ✨ `test_claim_respects_available_at` - Jobs with future available_at are skipped
4. ✨ `test_claim_priority_pending_retry_first` - PendingRetry claimed before Created
5. ✨ `test_orphan_recovery_timeout` - Stuck jobs are healed after timeout
6. ✨ `test_claim_keeps_claimed_by_during_processing` - claimed_by field retained
7. ✨ `test_claim_cleared_on_verification` - claimed_by cleared when moving to PendingVerification
8. ✨ `test_poll_interval_configuration` - Poll interval is configurable

### 2. Job Creation Cap Tests (`tests/workers/caps/`)
**Purpose:** Test max_concurrent_created_*_jobs caps

```rust
tests/workers/caps/
├── snos_caps.rs         // SNOS job creation capping
├── proving_caps.rs      // ProofCreation job creation capping
├── aggregator_caps.rs   // Aggregator job creation capping
└── mod.rs
```

**Tests:**
1. ✨ `test_snos_job_cap_respected` - SNOS jobs capped at config limit
2. ✨ `test_proving_job_cap_respected` - ProofCreation jobs capped
3. ✨ `test_aggregator_job_cap_respected` - Aggregator jobs capped
4. ✨ `test_cap_counts_created_and_pending_retry` - Both statuses counted
5. ✨ `test_cap_allows_creation_when_below_limit` - Jobs created when slots available

### 3. Orphan Healing Tests (`tests/workers/healing/`)
**Purpose:** Test orphan detection and recovery for ProofCreation and Aggregator

```rust
tests/workers/healing/
├── proving_healing.rs
├── aggregator_healing.rs
└── mod.rs
```

**Tests:**
1. ✨ `test_proving_orphan_healing` - Orphaned ProofCreation jobs are reset
2. ✨ `test_aggregator_orphan_healing` - Orphaned Aggregator jobs are reset
3. ✨ `test_healing_respects_timeout` - Only jobs past timeout are healed
4. ✨ `test_healing_clears_claim` - Healed jobs have claimed_by cleared

---

## Test Cleanup Checklist

### Phase 10: Update Existing Tests

#### A. Remove SQS Expectations (All Job Tests)
- [ ] Remove `queue.expect_send_message()` for job processing queues
- [ ] Remove `queue.expect_send_message()` for job verification queues
- [ ] Remove `consume_message_from_queue()` calls for job queues
- [ ] Remove imports: `QueueNameForJobType`, `process_queue_name()`, `verify_queue_name()`

#### B. Update Job Processing Tests
- [ ] `tests/jobs/mod.rs` - 23 tests
- [ ] `tests/jobs/snos_job/mod.rs` - 11 tests
- [ ] `tests/jobs/proving_job/mod.rs` - 6 tests
- [ ] `tests/jobs/aggregator_job/mod.rs` - 6 tests
- [ ] `tests/jobs/da_job/mod.rs` - 6 tests
- [ ] `tests/jobs/state_update_job/mod.rs` - 8 tests

#### C. Update Worker Tests
- [ ] `tests/workers/snos/mod.rs` - 4 tests
- [ ] `tests/workers/proving/mod.rs` - 2 tests
- [ ] `tests/workers/update_state/mod.rs` - 6 tests
- [ ] `tests/workers/batching/mod.rs` - 16 tests
- [ ] `tests/workers/batching/e2e.rs` - 2 tests

#### D. Update Server Tests
- [ ] `tests/server/job_routes.rs` - 12 tests
- [ ] `tests/server/admin_routes.rs` - 8 tests
- [ ] `tests/server/jobs_by_status.rs` - 2 tests

#### E. Update Queue Tests
- [ ] `tests/queue/mod.rs` - Keep only WorkerTrigger tests

#### F. Update Setup Tests
- [ ] `tests/setup/sqs.rs` - WorkerTrigger only

---

## Test Migration Strategy

### Option 1: Incremental Update (Recommended)
1. Start with database tests (verify they pass)
2. Update job tests one file at a time
3. Add new greedy-specific tests alongside
4. Update worker tests
5. Update server tests
6. Clean up queue/setup tests

### Option 2: Big Bang
1. Comment out all failing tests
2. Write new greedy test suite from scratch
3. Port over relevant test logic
4. Remove old tests

---

## Key Testing Principles for Greedy Mode

1. **Test Database State, Not Queues**
   - Verify job status in DB, not SQS messages
   - Check `claimed_by` and `available_at` fields

2. **Test Atomic Operations**
   - Use multiple concurrent claims to test atomicity
   - Verify only one orchestrator wins

3. **Test Polling Behavior**
   - Verify poll interval is respected
   - Verify `available_at` is honored
   - Verify priority ordering (PendingRetry first)

4. **Test Orphan Recovery**
   - Simulate timeouts
   - Verify healing resets jobs
   - Verify healing clears claims

5. **Test Job Creation Caps**
   - Verify cap counting is accurate
   - Verify slots are respected
   - Verify cap includes both Created and PendingRetry

---

## Estimated Test Count

| Category | Current | To Remove | To Update | New Tests | Final |
|----------|---------|-----------|-----------|-----------|-------|
| Database | 49 | 0 | 0 | 0 | 49 |
| Jobs | 60 | 0 | 60 | 5 | 65 |
| Workers | 30 | 0 | 30 | 20 | 50 |
| Server | 26 | 0 | 26 | 3 | 29 |
| Queue | 2 | 0 | 2 | 3 | 5 |
| Setup | 4 | 0 | 4 | 0 | 4 |
| Infrastructure | 11 | 0 | 0 | 0 | 11 |
| **Greedy (New)** | 0 | 0 | 0 | 20 | 20 |
| **Total** | **182** | **0** | **122** | **51** | **233** |

---

## Priority Order

1. **High Priority:** Database tests (verify baseline)
2. **High Priority:** Job tests (core functionality)
3. **High Priority:** New greedy tests (claiming, polling, caps)
4. **Medium Priority:** Worker tests (trigger logic)
5. **Medium Priority:** Server tests (API integration)
6. **Low Priority:** Queue/Setup tests (infrastructure)

---

## Notes

- All tests must work without SQS job queues
- WorkerTrigger queue should still be tested (EventBridge integration)
- Focus on DB state verification over queue message verification
- Greedy mode introduces new test scenarios (atomic claiming, orphan healing, caps)
- Tests should be parallelizable where possible (use unique job/batch IDs)
