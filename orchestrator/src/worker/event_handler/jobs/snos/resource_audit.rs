//! Post-execution telemetry for underestimated SNOS proving work. This is a batch resource
//! audit, not a receipt/fee audit: over-reservation by one transaction can hide another's deficit.
//! Use the batch's protocol constants (or the same override passed to SNOS) and Blockifier's
//! default proving builtin weights. Custom sequencer proving weights are not advertised by the
//! feeder gateway, so this model is explicitly named in logs. Output cells have a separate
//! capacity limit; they have no proving-gas price in this model.
#[cfg(test)]
mod tests;

use crate::core::client::database::{DatabaseClient, SnosBatchDbQuery};
use crate::types::batch::SnosBatch;
use crate::types::jobs::metadata::SnosMetadata;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use blockifier::blockifier_versioned_constants::{BaseGasCosts, BuiltinGasCosts, VersionedConstants};
use blockifier::bouncer::BuiltinWeights;
use blockifier::execution::call_info::CairoPrimitiveName;
use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::vm::runners::cairo_runner::ExecutionResources;
use color_eyre::eyre::{ensure, eyre, Result};
use opentelemetry::KeyValue;
use starknet_api::versioned_constants_logic::VersionedConstantsTrait;
use std::time::Duration;
use tracing::{error, info, warn};

/// Telemetry must not hold up an otherwise successful PIE when the batch store is unavailable.
const BATCH_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
struct GasComparison {
    actual: u64,
    reserved: u64,
}

#[derive(Debug)]
struct ResourceAudit {
    proving_gas: GasComparison,
    sierra_gas: GasComparison,
}

/// Records one outcome per resource and execution attempt, including explicit `unavailable` outcomes.
/// Neither a deficit nor a failed audit changes the job's success or settlement eligibility.
pub(super) async fn record(
    database: &dyn DatabaseClient,
    metadata: &SnosMetadata,
    resources: &ExecutionResources,
    constants_override: Option<&VersionedConstants>,
) -> &'static str {
    let result = tokio::time::timeout(BATCH_LOOKUP_TIMEOUT, async {
        let batches = database
            .get_snos_batches(SnosBatchDbQuery {
                indexes: Some(vec![metadata.snos_batch_index]),
                limit: Some(2),
                ..Default::default()
            })
            .await?;
        ensure!(batches.len() == 1, "Expected exactly one saved SNOS batch");
        compare(&batches[0], metadata, resources, constants_override)
    })
    .await;
    match result {
        Ok(Ok(audit)) => {
            let mut exceeded = false;
            for (resource, comparison) in [("proving_gas", audit.proving_gas), ("sierra_gas", audit.sierra_gas)] {
                let shortfall = comparison.actual.saturating_sub(comparison.reserved);
                info!(
                    batch_index = metadata.snos_batch_index,
                    start_block = metadata.start_block,
                    end_block = metadata.end_block,
                    resource,
                    cost_model = "blockifier_default_proving_and_protocol_sierra",
                    constants_override = constants_override.is_some(),
                    actual_gas = comparison.actual,
                    reserved_gas = comparison.reserved,
                    shortfall,
                    n_steps = resources.n_steps,
                    n_memory_holes = resources.n_memory_holes,
                    builtins = ?resources.builtin_instance_counter,
                    "SNOS resource audit"
                );
                let outcome = if shortfall > 0 {
                    exceeded = true;
                    error!(
                        batch_index = metadata.snos_batch_index,
                        start_block = metadata.start_block,
                        end_block = metadata.end_block,
                        resource,
                        actual_gas = comparison.actual,
                        reserved_gas = comparison.reserved,
                        shortfall,
                        "SNOS work exceeds Madara's batch reservation; possible undercharging"
                    );
                    "exceeded"
                } else {
                    "within_reservation"
                };
                record_outcome(resource, outcome);
            }
            if exceeded {
                "exceeded"
            } else {
                "within_reservation"
            }
        }
        failure => {
            warn!(batch_index = metadata.snos_batch_index, error = ?failure, "SNOS resource audit unavailable");
            for resource in ["proving_gas", "sierra_gas"] {
                record_outcome(resource, "unavailable");
            }
            "unavailable"
        }
    }
}

/// Keep labels bounded; retry attempts are observations, not distinct undercharged transactions.
fn record_outcome(resource: &'static str, outcome: &'static str) {
    ORCHESTRATOR_METRICS
        .snos_resource_audit_total
        .add(1, &[KeyValue::new("resource", resource), KeyValue::new("outcome", outcome)]);
}

/// Compare exactly the range executed by this job. The saved reservation already sums all
/// blocks and includes batching's configured empty-block allowance; do not add it a second time.
fn compare(
    batch: &SnosBatch,
    metadata: &SnosMetadata,
    resources: &ExecutionResources,
    constants_override: Option<&VersionedConstants>,
) -> Result<ResourceAudit> {
    ensure!(
        batch.index == metadata.snos_batch_index
            && batch.start_block == metadata.start_block
            && batch.end_block == metadata.end_block
            && batch.num_blocks == metadata.num_blocks
            && batch.end_block.checked_sub(batch.start_block).and_then(|span| span.checked_add(1))
                == Some(batch.num_blocks),
        "SNOS batch range does not match executed job"
    );
    let constants = match constants_override {
        Some(constants) => constants,
        None => VersionedConstants::get(&batch.starknet_version.to_string().try_into()?)?,
    };
    let gas_costs = &constants.os_constants.gas_costs;
    Ok(ResourceAudit {
        proving_gas: GasComparison {
            actual: resource_gas(resources, &gas_costs.base, &BuiltinWeights::default().gas_costs)?,
            reserved: batch.builtin_weights.proving_gas.0,
        },
        sierra_gas: GasComparison {
            actual: resource_gas(resources, &gas_costs.base, &gas_costs.builtins)?,
            reserved: batch.builtin_weights.sierra_gas.0,
        },
    })
}

/// The same steps + memory holes + builtin pricing used by Blockifier, with checked arithmetic
/// and fallible builtin lookup so telemetry cannot panic or silently undercount an unknown cost.
fn resource_gas(resources: &ExecutionResources, base: &BaseGasCosts, builtins: &BuiltinGasCosts) -> Result<u64> {
    let mut total = 0_u64;
    let mut add = |count: usize, price: u64| -> Result<()> {
        let cost = u64::try_from(count)?.checked_mul(price).ok_or_else(|| eyre!("Resource cost overflow"))?;
        total = total.checked_add(cost).ok_or_else(|| eyre!("Resource total overflow"))?;
        Ok(())
    };
    add(resources.n_steps, base.step_gas_cost)?;
    add(resources.n_memory_holes, base.memory_hole_gas_cost)?;
    for (&builtin, &count) in &resources.builtin_instance_counter {
        if builtin != BuiltinName::output && count != 0 {
            add(count, builtins.get_cairo_primitive_gas_cost(&CairoPrimitiveName::Builtin(builtin))?)?;
        }
    }
    Ok(total)
}
