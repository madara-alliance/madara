//! Minimal L2 gas limit discovery for fee estimation.
//!
//! Starknet 0.13.4 introduced runtime L2 gas accounting (for Sierra >= 1.7 contracts): execution
//! halts when the transaction's `l2_gas.max_amount` is exhausted, and the worst-case-path gas
//! requirements can exceed the actual consumption. Executing an estimation request once with the
//! user-provided bounds therefore fails with 'Out of gas' for the standard wallet pattern
//! (estimate with zero bounds), and even a successful run can report an `l2_gas_consumed` that is
//! *below* the minimal viable limit.
//!
//! Instead, estimation maxes out the L2 gas limit, executes once to learn the consumption, and
//! searches for the minimal limit that still executes successfully: consumption + 10% directly,
//! else a binary search between the consumption and the maximum. The algorithm and its constants
//! mirror pathfinder (`crates/executor/src/transaction.rs`,
//! `find_l2_gas_limit_and_execute_transaction`) and juno
//! (`vm/rust/src/entrypoint/execute/binary_search_execution.rs`).

use crate::{Error, ExecutionContext, TxExecError};
use blockifier::execution::contract_class::TrackedResource;
use blockifier::state::cached_state::TransactionalState;
use blockifier::state::state_api::StateReader;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::ExecutableTransaction;
use mc_db::MadaraStorageRead;
use starknet_api::core::ClassHash;
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use starknet_api::execution_resources::{GasAmount, GasVector};
use starknet_api::transaction::fields::{GasVectorComputationMode, ValidResourceBounds};

/// The revert string blockifier produces when execution runs out of Sierra gas.
const OUT_OF_GAS_CAIRO_STRING: &str = "0x4f7574206f6620676173 ('Out of gas')";

/// The binary search stops once the bounds are within this margin (same value as pathfinder).
const L2_GAS_SEARCH_MARGIN: GasAmount = GasAmount(1_000_000);

/// Buffer added to the consumed L2 gas before trying it as a limit: 10%, like both references.
fn with_estimation_buffer(gas: GasAmount) -> GasAmount {
    GasAmount(gas.0.saturating_add(gas.0 / 10))
}

/// Midpoint with ceiling: without it the search can loop forever when the bounds are 1 apart.
fn midpoint(lower: GasAmount, upper: GasAmount) -> GasAmount {
    GasAmount(lower.0 + (upper.0 - lower.0).div_ceil(2))
}

fn search_done(lower: GasAmount, upper: GasAmount) -> bool {
    upper.0 - lower.0 <= L2_GAS_SEARCH_MARGIN.0
}

fn is_deploy_account(tx: &Transaction) -> bool {
    matches!(tx, Transaction::Account(AccountTransaction { tx: ApiAccountTransaction::DeployAccount(_), .. }))
}

fn charges_fee(tx: &Transaction) -> bool {
    matches!(tx, Transaction::Account(tx) if tx.execution_flags.charge_fee)
}

/// Sets `l2_gas.max_amount` on a v3 all-resource-bounds account transaction.
fn set_l2_gas_limit(tx: &mut Transaction, gas_limit: GasAmount) -> Result<(), Error> {
    use starknet_api::transaction::{DeclareTransaction, DeployAccountTransaction, InvokeTransaction};

    let resource_bounds = match tx {
        Transaction::Account(AccountTransaction { tx: ApiAccountTransaction::Declare(tx), .. }) => match &mut tx.tx {
            DeclareTransaction::V3(tx) => Some(&mut tx.resource_bounds),
            _ => None,
        },
        Transaction::Account(AccountTransaction { tx: ApiAccountTransaction::DeployAccount(tx), .. }) => {
            match &mut tx.tx {
                DeployAccountTransaction::V3(tx) => Some(&mut tx.resource_bounds),
                _ => None,
            }
        }
        Transaction::Account(AccountTransaction { tx: ApiAccountTransaction::Invoke(tx), .. }) => match &mut tx.tx {
            InvokeTransaction::V3(tx) => Some(&mut tx.resource_bounds),
            _ => None,
        },
        Transaction::L1Handler(_) => None,
    };

    if let Some(ValidResourceBounds::AllResources(all_resources)) = resource_bounds {
        all_resources.l2_gas.max_amount = gas_limit;
        return Ok(());
    }
    Err(Error::Internal(anyhow::anyhow!("set_l2_gas_limit called on a transaction without all-resource bounds")))
}

enum ProbeOutcome {
    /// Executed without running out of L2 gas (it may still have reverted for another reason).
    Success(Box<TransactionExecutionInfo>),
    /// Ran out of L2 gas, either as a revert or as a hard validation error.
    OutOfGas,
}

impl<D: MadaraStorageRead> ExecutionContext<D> {
    /// L2 gas accounting only takes effect when the transaction declares all-resource bounds and
    /// the sender class is tracked by Sierra gas (compiled as Sierra >= 1.7). Mirrors pathfinder's
    /// `l2_gas_accounting_enabled`: only the sender class is checked, the 10% estimation buffer
    /// covers mixed-version call trees.
    pub(crate) fn l2_gas_accounting_enabled(
        &mut self,
        tx: &Transaction,
        gas_vector_computation_mode: &GasVectorComputationMode,
    ) -> Result<bool, Error> {
        if gas_vector_computation_mode != &GasVectorComputationMode::All {
            return Ok(false);
        }
        if is_deploy_account(tx) {
            return Ok(true);
        }

        let sender_class_hash = self
            .state
            .get_class_hash_at(tx.sender_address())
            .map_err(TransactionExecutionError::StateError)
            .map_err(|err| Error::Internal(anyhow::anyhow!("Getting sender class hash: {err:#}")))?;
        // Sender not deployed yet: nothing to look up, the execution will fail anyway.
        if sender_class_hash == ClassHash::default() {
            return Ok(false);
        }

        let tracked_resource = self
            .state
            .get_compiled_class(sender_class_hash)
            .map_err(|err| Error::Internal(anyhow::anyhow!("Getting sender compiled class: {err:#}")))?
            .tracked_resource(&self.block_context.versioned_constants().min_sierra_version_for_sierra_gas, None);

        Ok(tracked_resource == TrackedResource::SierraGas)
    }

    /// Executes `tx` on a discarded fork of the current state.
    fn probe_l2_gas_limit(&mut self, tx: &Transaction) -> Result<ProbeOutcome, TransactionExecutionError> {
        let mut transactional_state = TransactionalState::create_transactional(&mut self.state);
        let result = tx.execute_raw(&mut transactional_state, &self.block_context, false);
        transactional_state.abort();

        match result {
            Ok(info) => {
                let out_of_gas = info
                    .revert_error
                    .as_ref()
                    .is_some_and(|revert_error| revert_error.to_string().contains(OUT_OF_GAS_CAIRO_STRING));
                if out_of_gas {
                    Ok(ProbeOutcome::OutOfGas)
                } else {
                    Ok(ProbeOutcome::Success(Box::new(info)))
                }
            }
            // Out of gas during validation surfaces as a hard error, not a revert.
            Err(err) if err.to_string().contains(OUT_OF_GAS_CAIRO_STRING) => Ok(ProbeOutcome::OutOfGas),
            Err(err) => Err(err),
        }
    }

    /// Finds the minimal L2 gas limit (within [`L2_GAS_SEARCH_MARGIN`]) that lets `tx` execute
    /// without running out of L2 gas, and leaves that limit set on `tx` so the caller's final
    /// execution runs with it. Returns the gas vector to use for the fee estimate: the consumed
    /// gas of the max-limit run with `l2_gas` replaced by the discovered limit.
    ///
    /// Only call this for fee estimation/simulation without fee charge: the maximum is the
    /// protocol execution cap, not the amount covered by the account balance.
    pub(crate) fn find_l2_gas_limit(
        &mut self,
        tx: &mut Transaction,
        make_err: &impl Fn(TransactionExecutionError) -> TxExecError,
    ) -> Result<Option<GasVector>, Error> {
        debug_assert!(!charges_fee(tx));

        let max_l2_gas_limit = self.block_context.versioned_constants().os_constants.execute_max_sierra_gas;
        set_l2_gas_limit(tx, max_l2_gas_limit)?;

        let info = match self.probe_l2_gas_limit(tx).map_err(make_err)? {
            ProbeOutcome::Success(info) => info,
            // Even the protocol cap is not enough: leave the limit at the maximum and let the
            // caller's execution surface the out-of-gas revert.
            ProbeOutcome::OutOfGas => return Ok(None),
        };
        // Reverted for a reason other than gas: no point searching, the caller's execution
        // reports the revert.
        if info.is_reverted() {
            return Ok(None);
        }

        let l2_gas_consumed = info.receipt.gas.l2_gas;
        let adjusted = with_estimation_buffer(l2_gas_consumed).min(max_l2_gas_limit);

        set_l2_gas_limit(tx, adjusted)?;
        let gas_limit = match self.probe_l2_gas_limit(tx).map_err(make_err)? {
            // Consumption + 10% is enough: use it and skip the binary search.
            ProbeOutcome::Success(_) => adjusted,
            ProbeOutcome::OutOfGas => {
                let mut lower_bound = l2_gas_consumed;
                let mut upper_bound = max_l2_gas_limit;
                let mut current = midpoint(lower_bound, upper_bound);
                loop {
                    tracing::debug!(
                        lower_bound = lower_bound.0,
                        upper_bound = upper_bound.0,
                        current = current.0,
                        "Searching for minimal L2 gas limit"
                    );
                    set_l2_gas_limit(tx, current)?;
                    match self.probe_l2_gas_limit(tx).map_err(make_err)? {
                        ProbeOutcome::Success(_) => {
                            if search_done(lower_bound, upper_bound) {
                                break current;
                            }
                            upper_bound = current;
                        }
                        ProbeOutcome::OutOfGas => lower_bound = current,
                    }
                    current = midpoint(lower_bound, upper_bound);
                }
            }
        };

        set_l2_gas_limit(tx, gas_limit)?;
        Ok(Some(GasVector { l2_gas: gas_limit, ..info.receipt.gas }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_uses_ceiling() {
        assert_eq!(midpoint(GasAmount(0), GasAmount(10)), GasAmount(5));
        // Without ceiling this would return the lower bound forever.
        assert_eq!(midpoint(GasAmount(9), GasAmount(10)), GasAmount(10));
        assert_eq!(midpoint(GasAmount(10), GasAmount(10)), GasAmount(10));
    }

    #[test]
    fn search_done_within_margin() {
        assert!(search_done(GasAmount(0), GasAmount(1_000_000)));
        assert!(!search_done(GasAmount(0), GasAmount(1_000_001)));
    }

    #[test]
    fn estimation_buffer_is_ten_percent() {
        assert_eq!(with_estimation_buffer(GasAmount(100)), GasAmount(110));
        assert_eq!(with_estimation_buffer(GasAmount(u64::MAX)), GasAmount(u64::MAX));
    }
}
