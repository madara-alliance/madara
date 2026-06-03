use chrono::Utc;
use dashmap::{mapref::entry::Entry, DashMap};
use once_cell::sync::Lazy;
use opentelemetry::metrics::{Meter, ObservableGauge};
use opentelemetry::KeyValue;
use std::time::Instant;

use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::jobs::WorkerTriggerType;
use crate::types::queue::{JobState, QueueType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;

// Keep descriptors at zero so the async gauge can publish drained workloads as 0 on later scrapes.
static ACTIVE_WORKLOAD_SLOTS: Lazy<DashMap<WorkloadDescriptor, u64>> = Lazy::new(DashMap::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum WorkKind {
    SnosRun,
    ProofCreation,
    ProofRegistration,
    DataSubmission,
    StateTransition,
    Aggregator,
    SnosBatching,
    AggregatorBatching,
    StorageCleanup,
    Healing,
}

impl WorkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::SnosRun => "SnosRun",
            Self::ProofCreation => "ProofCreation",
            Self::ProofRegistration => "ProofRegistration",
            Self::DataSubmission => "DataSubmission",
            Self::StateTransition => "StateTransition",
            Self::Aggregator => "Aggregator",
            Self::SnosBatching => "SnosBatching",
            Self::AggregatorBatching => "AggregatorBatching",
            Self::StorageCleanup => "StorageCleanup",
            Self::Healing => "Healing",
        }
    }

    fn from_job_type(job_type: &JobType) -> Self {
        match job_type {
            JobType::SnosRun => Self::SnosRun,
            JobType::ProofCreation => Self::ProofCreation,
            JobType::ProofRegistration => Self::ProofRegistration,
            JobType::DataSubmission => Self::DataSubmission,
            JobType::StateTransition => Self::StateTransition,
            JobType::Aggregator => Self::Aggregator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum WorkPhase {
    // Concrete lifecycle stage shown on dashboards.
    Process,
    Verify,
    Trigger,
    Maintenance,
}

impl WorkPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Verify => "verify",
            Self::Trigger => "trigger",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum WorkloadOutcome {
    Success,
    Error,
}

impl WorkloadOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct WorkloadDescriptor {
    work_kind: WorkKind,
    work_phase: WorkPhase,
    source_job_type: Option<WorkKind>,
}

impl WorkloadDescriptor {
    fn new(work_kind: WorkKind, work_phase: WorkPhase, source_job_type: Option<WorkKind>) -> Self {
        Self { work_kind, work_phase, source_job_type }
    }

    fn active_attributes(self) -> [KeyValue; 3] {
        [
            KeyValue::new("work_kind", self.work_kind.as_str()),
            KeyValue::new("work_phase", self.work_phase.as_str()),
            KeyValue::new("source_job_type", self.source_job_type.map_or("none", WorkKind::as_str)),
        ]
    }

    fn completed_attributes(self, outcome: WorkloadOutcome) -> [KeyValue; 4] {
        [
            KeyValue::new("work_kind", self.work_kind.as_str()),
            KeyValue::new("work_phase", self.work_phase.as_str()),
            KeyValue::new("source_job_type", self.source_job_type.map_or("none", WorkKind::as_str)),
            KeyValue::new("outcome", outcome.as_str()),
        ]
    }
}

pub struct WorkloadTracker {
    descriptor: WorkloadDescriptor,
    started_at: Instant,
    finished: bool,
}

impl WorkloadTracker {
    fn start(descriptor: WorkloadDescriptor) -> Self {
        increment_active_slots(descriptor);
        Self { descriptor, started_at: Instant::now(), finished: false }
    }

    pub fn finish_success(mut self) {
        self.finish(WorkloadOutcome::Success);
    }

    pub fn finish_error(mut self) {
        self.finish(WorkloadOutcome::Error);
    }

    fn finish(&mut self, outcome: WorkloadOutcome) {
        if self.finished {
            return;
        }

        self.finished = true;
        let duration_seconds = self.started_at.elapsed().as_secs_f64();
        let completed_attributes = self.descriptor.completed_attributes(outcome);

        ORCHESTRATOR_METRICS.workload_busy_seconds_total.add(duration_seconds, &completed_attributes);

        decrement_active_slots(self.descriptor);
    }
}

impl Drop for WorkloadTracker {
    fn drop(&mut self) {
        self.finish(WorkloadOutcome::Error);
    }
}

fn increment_active_slots(descriptor: WorkloadDescriptor) {
    match ACTIVE_WORKLOAD_SLOTS.entry(descriptor) {
        Entry::Occupied(mut entry) => {
            let value = entry.get_mut();
            *value += 1;
        }
        Entry::Vacant(entry) => {
            entry.insert(1);
        }
    }
}

fn decrement_active_slots(descriptor: WorkloadDescriptor) {
    match ACTIVE_WORKLOAD_SLOTS.entry(descriptor) {
        Entry::Occupied(mut entry) => {
            let value = entry.get_mut();
            *value = value.saturating_sub(1);
        }
        Entry::Vacant(entry) => {
            entry.insert(0);
        }
    }
}

pub fn register_workload_active_slots_observer(meter: &Meter) -> ObservableGauge<u64> {
    meter
        .u64_observable_gauge("workload_active_slots")
        .with_description("Current active workload slots by kind and phase")
        .with_unit("slots")
        .with_callback(|observer| {
            for active_slots in ACTIVE_WORKLOAD_SLOTS.iter() {
                observer.observe(*active_slots.value(), &active_slots.key().active_attributes());
            }
        })
        .build()
}

fn workload_descriptor_for_job(job_type: &JobType, job_state: JobState) -> WorkloadDescriptor {
    let work_kind = WorkKind::from_job_type(job_type);
    match job_state {
        JobState::Processing => WorkloadDescriptor::new(work_kind, WorkPhase::Process, None),
        JobState::Verification => WorkloadDescriptor::new(work_kind, WorkPhase::Verify, None),
    }
}

fn workload_descriptor_for_worker_trigger(worker_trigger_type: &WorkerTriggerType) -> WorkloadDescriptor {
    match worker_trigger_type {
        WorkerTriggerType::Snos => WorkloadDescriptor::new(WorkKind::SnosRun, WorkPhase::Trigger, None),
        WorkerTriggerType::Proving => WorkloadDescriptor::new(WorkKind::ProofCreation, WorkPhase::Trigger, None),
        WorkerTriggerType::ProofRegistration => {
            WorkloadDescriptor::new(WorkKind::ProofRegistration, WorkPhase::Trigger, None)
        }
        WorkerTriggerType::DataSubmission => {
            WorkloadDescriptor::new(WorkKind::DataSubmission, WorkPhase::Trigger, None)
        }
        WorkerTriggerType::UpdateState => WorkloadDescriptor::new(WorkKind::StateTransition, WorkPhase::Trigger, None),
        WorkerTriggerType::Aggregator => WorkloadDescriptor::new(WorkKind::Aggregator, WorkPhase::Trigger, None),
        WorkerTriggerType::AggregatorBatching => {
            WorkloadDescriptor::new(WorkKind::AggregatorBatching, WorkPhase::Trigger, None)
        }
        WorkerTriggerType::SnosBatching => WorkloadDescriptor::new(WorkKind::SnosBatching, WorkPhase::Trigger, None),
        WorkerTriggerType::StorageCleanup => {
            WorkloadDescriptor::new(WorkKind::StorageCleanup, WorkPhase::Maintenance, None)
        }
    }
}

fn workload_descriptor_for_queue(queue_type: &QueueType) -> Option<WorkloadDescriptor> {
    match queue_type {
        QueueType::SnosJobProcessing => Some(workload_descriptor_for_job(&JobType::SnosRun, JobState::Processing)),
        QueueType::SnosJobVerification => Some(workload_descriptor_for_job(&JobType::SnosRun, JobState::Verification)),
        QueueType::ProvingJobProcessing => {
            Some(workload_descriptor_for_job(&JobType::ProofCreation, JobState::Processing))
        }
        QueueType::ProvingJobVerification => {
            Some(workload_descriptor_for_job(&JobType::ProofCreation, JobState::Verification))
        }
        QueueType::ProofRegistrationJobProcessing => {
            Some(workload_descriptor_for_job(&JobType::ProofRegistration, JobState::Processing))
        }
        QueueType::ProofRegistrationJobVerification => {
            Some(workload_descriptor_for_job(&JobType::ProofRegistration, JobState::Verification))
        }
        QueueType::DataSubmissionJobProcessing => {
            Some(workload_descriptor_for_job(&JobType::DataSubmission, JobState::Processing))
        }
        QueueType::DataSubmissionJobVerification => {
            Some(workload_descriptor_for_job(&JobType::DataSubmission, JobState::Verification))
        }
        QueueType::UpdateStateJobProcessing => {
            Some(workload_descriptor_for_job(&JobType::StateTransition, JobState::Processing))
        }
        QueueType::UpdateStateJobVerification => {
            Some(workload_descriptor_for_job(&JobType::StateTransition, JobState::Verification))
        }
        QueueType::AggregatorJobProcessing => {
            Some(workload_descriptor_for_job(&JobType::Aggregator, JobState::Processing))
        }
        QueueType::AggregatorJobVerification => {
            Some(workload_descriptor_for_job(&JobType::Aggregator, JobState::Verification))
        }
        // Capacity is intentionally limited to queue-backed processing/verification work.
        // Trigger, failure-handling, priority helper, and maintenance-style workloads still emit
        // active/busy signals for observability, but they do not have a meaningful slot-capacity
        // denominator for the sizing questions this PR is targeting today.
        QueueType::WorkerTrigger
        | QueueType::JobHandleFailure
        | QueueType::PriorityProcessingQueue
        | QueueType::PriorityVerificationQueue => None,
    }
}

fn healing_descriptor(source_job_type: &JobType) -> WorkloadDescriptor {
    WorkloadDescriptor::new(WorkKind::Healing, WorkPhase::Maintenance, Some(WorkKind::from_job_type(source_job_type)))
}

/// Helper functions to record metrics at various points in the job lifecycle
/// These should be called from the existing service handlers without modifying the DB model
pub struct MetricsRecorder;

impl MetricsRecorder {
    pub fn start_job_workload(job_type: &JobType, job_state: JobState) -> WorkloadTracker {
        WorkloadTracker::start(workload_descriptor_for_job(job_type, job_state))
    }

    pub fn start_worker_trigger_workload(worker_trigger_type: &WorkerTriggerType) -> WorkloadTracker {
        WorkloadTracker::start(workload_descriptor_for_worker_trigger(worker_trigger_type))
    }

    pub fn start_healing_workload(job_type: &JobType) -> WorkloadTracker {
        WorkloadTracker::start(healing_descriptor(job_type))
    }

    pub fn record_workload_capacity_for_queue(queue_type: &QueueType, max_slots: usize) {
        if let Some(descriptor) = workload_descriptor_for_queue(queue_type) {
            ACTIVE_WORKLOAD_SLOTS.entry(descriptor).or_insert(0);
            ORCHESTRATOR_METRICS.workload_capacity_slots.record(max_slots as f64, &descriptor.active_attributes());
        }
    }

    pub fn record_healed_job(job_type: &JobType) {
        ORCHESTRATOR_METRICS
            .healed_jobs_total
            .add(1.0, &[KeyValue::new("source_job_type", WorkKind::from_job_type(job_type).as_str())]);
    }

    /// Record metrics when a job is created and enters the queue
    pub fn record_job_created(job: &JobItem) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "create_job"),
        ];

        // Record that a job entered the queue
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);

        // TODO: Query DB to get current queue depth for this job type
        // This would require async context - implement in service layer
    }

    pub fn record_successful_job_operation(count: f64, attributes: &[KeyValue]) {
        ORCHESTRATOR_METRICS.successful_job_operations.add(count, attributes);
    }

    pub fn record_failed_job_operation(count: f64, attributes: &[KeyValue]) {
        ORCHESTRATOR_METRICS.failed_job_operations.add(count, attributes);
    }

    pub fn record_job_response_time(duration_seconds: f64, attributes: &[KeyValue]) {
        ORCHESTRATOR_METRICS.jobs_response_time.record(duration_seconds, attributes);
    }

    pub fn record_verification_time(job_type: &JobType, duration_ms: f64) {
        ORCHESTRATOR_METRICS
            .verification_time
            .record(duration_ms, &[KeyValue::new("operation_job_type", format!("{:?}", job_type))]);
    }

    pub fn record_block_gauge(block_number: f64, attributes: &[KeyValue]) {
        ORCHESTRATOR_METRICS.block_gauge.record(block_number, attributes);
    }

    pub fn record_job_state_transition(from_state: JobStatus, to_state: JobStatus, job_type: &JobType) {
        ORCHESTRATOR_METRICS.job_state_transitions.add(
            1.0,
            &[
                KeyValue::new("from_state", from_state.to_string()),
                KeyValue::new("to_state", to_state.to_string()),
                KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            ],
        );
    }

    pub fn record_job_status(job: &JobItem, status: &JobStatus) {
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            job.internal_id,
            &job.job_type,
            status,
            &job.id.to_string(),
        );
    }

    pub fn record_db_call(duration_seconds: f64, attributes: &[KeyValue]) {
        ORCHESTRATOR_METRICS.db_calls_response_time.record(duration_seconds, attributes);
    }

    /// Record metrics when a job starts processing
    pub fn record_job_processing_started(job: &JobItem, queue_wait_time_seconds: f64) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "process_job"),
        ];

        // Record queue wait time
        ORCHESTRATOR_METRICS.job_queue_wait_time.record(queue_wait_time_seconds, &attributes);
    }

    /// Record metrics when a job is retried
    pub fn record_job_retry(job: &JobItem, retry_reason: &str) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("retry_reason", retry_reason.to_string()),
        ];

        ORCHESTRATOR_METRICS.job_retry_count.add(1.0, &attributes);
    }

    /// Record metrics when job verification starts
    pub fn record_verification_started(job: &JobItem) {
        Self::record_job_state_transition(
            JobStatus::LockedForProcessing,
            JobStatus::PendingVerification,
            &job.job_type,
        );
    }

    /// Record metrics when a job completes successfully
    pub fn record_job_completed(job: &JobItem, e2e_duration_seconds: f64) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_job_status", "Completed"),
        ];

        // Record E2E latency
        ORCHESTRATOR_METRICS.job_e2e_latency.record(e2e_duration_seconds, &attributes);

        // Record successful completion
        Self::record_successful_job_operation(1.0, &attributes);

        // Record state transition
        Self::record_job_state_transition(JobStatus::PendingVerification, JobStatus::Completed, &job.job_type);
    }

    /// Record metrics when a job fails
    pub fn record_job_failed(job: &JobItem, _failure_reason: &str) {
        let attributes = [KeyValue::new("operation_job_type", format!("{:?}", job.job_type))];

        ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.failed_jobs.add(1.0, &attributes);
    }

    pub fn record_failed_job_total(job_type: &JobType, count: f64) {
        ORCHESTRATOR_METRICS.failed_jobs.add(count, &[KeyValue::new("operation_job_type", format!("{:?}", job_type))]);
    }

    /// Record metrics when a job times out
    pub fn record_job_timeout(job: &JobItem) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("timeout_type", "verification"),
        ];

        ORCHESTRATOR_METRICS.job_timeout_count.add(1.0, &attributes);
    }

    /// Record metrics when a job is abandoned after max retries
    pub fn record_job_abandoned(job: &JobItem, retry_count: i32) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("final_retry_count", retry_count.to_string()),
        ];

        ORCHESTRATOR_METRICS.job_abandoned_count.add(1.0, &attributes);
    }

    /// Record dependency wait time
    pub fn record_dependency_wait(job: &JobItem, wait_time_seconds: f64) {
        let attributes = [KeyValue::new("operation_job_type", format!("{:?}", job.job_type))];

        ORCHESTRATOR_METRICS.dependency_wait_time.record(wait_time_seconds, &attributes);
    }

    /// Record proof generation time
    pub fn record_proof_generation_time(proof_type: &str, duration_seconds: f64) {
        let attributes = [KeyValue::new("proof_type", proof_type.to_string())];

        ORCHESTRATOR_METRICS.proof_generation_time.record(duration_seconds, &attributes);
    }

    pub fn record_snos_job_processing_time(duration_seconds: f64) {
        let attributes = [KeyValue::new("operation_job_type", format!("{:?}", JobType::SnosRun))];
        ORCHESTRATOR_METRICS.snos_job_processing_time.record(duration_seconds, &attributes);
    }

    /// Record settlement time
    pub fn record_settlement_time(job_type: &JobType, duration_seconds: f64) {
        let attributes =
            [KeyValue::new("operation_job_type", format!("{:?}", job_type)), KeyValue::new("settlement_layer", "L1")];

        ORCHESTRATOR_METRICS.settlement_time.record(duration_seconds, &attributes);
    }

    /// Record active jobs count (should be called when jobs change state)
    pub async fn record_active_jobs(count: f64) {
        let attributes = [KeyValue::new("status", "processing")];

        ORCHESTRATOR_METRICS.active_jobs_count.record(count, &attributes);
    }

    /// Record parallelism factor
    pub async fn record_parallelism_factor(factor: f64) {
        let attributes = [];

        ORCHESTRATOR_METRICS.job_parallelism_factor.record(factor, &attributes);
    }

    /// Check and record SLA breaches
    pub fn check_and_record_sla_breach(job: &JobItem, max_e2e_seconds: i64, sla_type: &str) {
        let age_seconds = Utc::now().signed_duration_since(job.created_at).num_seconds();

        if age_seconds > max_e2e_seconds {
            let attributes = [
                KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
                KeyValue::new("sla_type", sla_type.to_string()),
            ];

            ORCHESTRATOR_METRICS.sla_breach_count.add(1.0, &attributes);
        }
    }

    /// Record orphaned job detection
    pub fn record_orphaned_job(job: &JobItem) {
        let attributes = [KeyValue::new("operation_job_type", format!("{:?}", job.job_type))];

        ORCHESTRATOR_METRICS.orphaned_jobs.add(1.0, &attributes);
    }

    /// Record Atlantic API call metrics
    pub fn record_atlantic_api_call(
        operation: &str,
        duration_seconds: f64,
        data_size_bytes: u64,
        success: bool,
        retry_count: u32,
        error_type: Option<&str>,
    ) {
        // Record call duration
        let duration_attrs =
            [KeyValue::new("operation", operation.to_string()), KeyValue::new("success", success.to_string())];
        ORCHESTRATOR_METRICS.atlantic_api_call_duration.record(duration_seconds, &duration_attrs);

        // Record total calls
        let call_attrs = [
            KeyValue::new("operation", operation.to_string()),
            KeyValue::new("status", if success { "success" } else { "error" }),
        ];
        ORCHESTRATOR_METRICS.atlantic_api_calls_total.add(1.0, &call_attrs);

        // Record errors if any
        if !success {
            // Ensure error_type is provided for failures - "unknown" indicates a bug in the caller
            let error_type_value = error_type.unwrap_or_else(|| {
                tracing::warn!(
                    operation = operation,
                    "Atlantic API failure recorded without error_type - this indicates a bug"
                );
                "unknown"
            });
            let error_attrs = [
                KeyValue::new("operation", operation.to_string()),
                KeyValue::new("error_type", error_type_value.to_string()),
            ];
            ORCHESTRATOR_METRICS.atlantic_api_errors_total.add(1.0, &error_attrs);
        }

        // Record retries if any
        if retry_count > 0 {
            let retry_attrs = [KeyValue::new("operation", operation.to_string())];
            ORCHESTRATOR_METRICS.atlantic_api_retries_total.add(retry_count as f64, &retry_attrs);
        }

        // Record data transfer
        if data_size_bytes > 0 {
            let data_attrs = [KeyValue::new("operation", operation.to_string()), KeyValue::new("direction", "request")];
            ORCHESTRATOR_METRICS.atlantic_data_transfer_bytes.add(data_size_bytes as f64, &data_attrs);
        }
    }

    /// Record Atlantic API response data size
    pub fn record_atlantic_response_size(operation: &str, data_size_bytes: u64) {
        if data_size_bytes > 0 {
            let attrs = [KeyValue::new("operation", operation.to_string()), KeyValue::new("direction", "response")];
            ORCHESTRATOR_METRICS.atlantic_data_transfer_bytes.add(data_size_bytes as f64, &attrs);
        }
    }

    // =============================================================================
    // Storage Cleanup Metrics
    // =============================================================================

    pub fn record_cleanup_run() {
        ORCHESTRATOR_METRICS.cleanup_runs_total.add(1.0, &[]);
    }

    pub fn record_cleanup_job_attempted() {
        ORCHESTRATOR_METRICS.cleanup_jobs_attempted.add(1.0, &[]);
    }

    pub fn record_cleanup_job_processed() {
        ORCHESTRATOR_METRICS.cleanup_jobs_processed.add(1.0, &[]);
    }

    pub fn record_cleanup_artifacts_tagged(count: f64) {
        if count > 0.0 {
            ORCHESTRATOR_METRICS.cleanup_artifacts_tagged.add(count, &[]);
        }
    }

    pub fn record_cleanup_failure(reason: &str) {
        ORCHESTRATOR_METRICS.cleanup_failures_total.add(1.0, &[KeyValue::new("reason", reason.to_string())]);
    }

    // Local aggregation metrics (SHARP / Mock paths)

    pub fn record_aggregator_run(prover: &str, duration_seconds: f64, success: bool) {
        let attrs = [KeyValue::new("prover", prover.to_string()), KeyValue::new("success", success.to_string())];
        ORCHESTRATOR_METRICS.aggregator_local_run_duration.record(duration_seconds, &attrs);
        ORCHESTRATOR_METRICS.aggregator_local_run_total.add(1.0, &attrs);
    }

    pub fn record_aggregator_child_count(prover: &str, count: usize) {
        ORCHESTRATOR_METRICS
            .aggregator_child_count
            .record(count as f64, &[KeyValue::new("prover", prover.to_string())]);
    }

    pub fn record_aggregator_failure(prover: &str, stage: &str, error_type: &str) {
        ORCHESTRATOR_METRICS.aggregator_local_run_failures_total.add(
            1.0,
            &[
                KeyValue::new("prover", prover.to_string()),
                KeyValue::new("stage", stage.to_string()),
                KeyValue::new("error_type", error_type.to_string()),
            ],
        );
    }

    pub fn record_aggregator_artifact_sizes(
        prover: &str,
        program_output_bytes: usize,
        da_segment_bytes: usize,
        pie_zip_bytes: usize,
    ) {
        let attrs = [KeyValue::new("prover", prover.to_string())];
        ORCHESTRATOR_METRICS.aggregator_program_output_bytes.record(program_output_bytes as f64, &attrs);
        ORCHESTRATOR_METRICS.aggregator_da_segment_bytes.record(da_segment_bytes as f64, &attrs);
        ORCHESTRATOR_METRICS.aggregator_pie_zip_bytes.record(pie_zip_bytes as f64, &attrs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_active_workload_state() {
        ACTIVE_WORKLOAD_SLOTS.clear();
    }

    fn active_slots_for(descriptor: WorkloadDescriptor) -> u64 {
        ACTIVE_WORKLOAD_SLOTS.get(&descriptor).map(|entry| *entry).unwrap_or(0)
    }

    #[test]
    fn job_processing_descriptor_is_mapped_consistently() {
        let descriptor = workload_descriptor_for_job(&JobType::SnosRun, JobState::Processing);

        assert_eq!(descriptor.work_kind, WorkKind::SnosRun);
        assert_eq!(descriptor.work_phase, WorkPhase::Process);
        assert_eq!(descriptor.source_job_type, None);
    }

    #[test]
    fn worker_trigger_descriptor_maps_storage_cleanup_to_maintenance() {
        let descriptor = workload_descriptor_for_worker_trigger(&WorkerTriggerType::StorageCleanup);

        assert_eq!(descriptor.work_kind, WorkKind::StorageCleanup);
        assert_eq!(descriptor.work_phase, WorkPhase::Maintenance);
    }

    #[test]
    fn capacity_descriptor_exists_only_for_queue_backed_workloads() {
        assert!(workload_descriptor_for_queue(&QueueType::SnosJobProcessing).is_some());
        assert!(workload_descriptor_for_queue(&QueueType::AggregatorJobVerification).is_some());
        assert!(workload_descriptor_for_queue(&QueueType::WorkerTrigger).is_none());
        assert!(workload_descriptor_for_queue(&QueueType::JobHandleFailure).is_none());
    }

    #[test]
    fn workload_tracker_increments_and_clears_active_slots() {
        reset_active_workload_state();
        let descriptor = workload_descriptor_for_job(&JobType::ProofCreation, JobState::Verification);

        let tracker = WorkloadTracker::start(descriptor);
        assert_eq!(active_slots_for(descriptor), 1);

        tracker.finish_success();
        assert_eq!(active_slots_for(descriptor), 0);
    }

    #[test]
    fn healing_descriptor_carries_source_job_type() {
        let descriptor = healing_descriptor(&JobType::StateTransition);

        assert_eq!(descriptor.work_kind, WorkKind::Healing);
        assert_eq!(descriptor.work_phase, WorkPhase::Maintenance);
        assert_eq!(descriptor.source_job_type, Some(WorkKind::StateTransition));
    }
}
