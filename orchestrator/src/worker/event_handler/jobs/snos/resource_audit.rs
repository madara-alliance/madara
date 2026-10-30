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

/// Records one outcome per execution attempt, including explicit `unavailable` outcomes.
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
    let outcome = match result {
        Ok(Ok((actual, reserved))) => {
            let shortfall = actual.saturating_sub(reserved);
            info!(
                batch_index = metadata.snos_batch_index,
                start_block = metadata.start_block,
                end_block = metadata.end_block,
                cost_model = "blockifier_default_proving",
                constants_override = constants_override.is_some(),
                actual_proving_gas = actual,
                reserved_proving_gas = reserved,
                shortfall,
                n_steps = resources.n_steps,
                n_memory_holes = resources.n_memory_holes,
                builtins = ?resources.builtin_instance_counter,
                "SNOS proving resource audit"
            );
            if shortfall > 0 {
                error!(
                    batch_index = metadata.snos_batch_index,
                    start_block = metadata.start_block,
                    end_block = metadata.end_block,
                    actual_proving_gas = actual,
                    reserved_proving_gas = reserved,
                    shortfall,
                    "SNOS proving work exceeds Madara's batch reservation; possible undercharging"
                );
                "exceeded"
            } else {
                "within_reservation"
            }
        }
        failure => {
            warn!(batch_index = metadata.snos_batch_index, error = ?failure, "SNOS resource audit unavailable");
            "unavailable"
        }
    };
    // No batch IDs or block heights in metric labels. Retries count as separate attempts.
    ORCHESTRATOR_METRICS.snos_resource_audit_total.add(1, &[KeyValue::new("outcome", outcome)]);
    outcome
}

/// Compare exactly the range executed by this job. The saved reservation already sums all
/// blocks and includes batching's configured empty-block allowance; do not add it a second time.
fn compare(
    batch: &SnosBatch,
    metadata: &SnosMetadata,
    resources: &ExecutionResources,
    constants_override: Option<&VersionedConstants>,
) -> Result<(u64, u64)> {
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
        None => VersionedConstants::get(&batch.starknet_version.to_string().parse()?)?,
    };
    let actual = proving_gas(resources, &constants.os_constants.gas_costs.base, &BuiltinWeights::default().gas_costs)?;
    Ok((actual, batch.builtin_weights.proving_gas.0))
}

/// The same steps + memory holes + builtin pricing used by Blockifier, with checked arithmetic
/// and fallible builtin lookup so telemetry cannot panic or silently undercount an unknown cost.
fn proving_gas(resources: &ExecutionResources, base: &BaseGasCosts, builtins: &BuiltinGasCosts) -> Result<u64> {
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
