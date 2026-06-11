use crate::{CallContractError, Error, ExecutionContext};
use blockifier::context::TransactionContext;
use blockifier::execution::entry_point::{
    CallEntryPoint, CallType, EntryPointExecutionContext, SierraGasRevertTracker,
};
use blockifier::execution::errors::{EntryPointExecutionError, PreExecutionError};
use blockifier::execution::stack_trace::{extract_trailing_cairo1_revert_trace, Cairo1RevertHeader};
use blockifier::execution::syscalls::hint_processor::ENTRYPOINT_NOT_FOUND_ERROR_FELT;
use blockifier::state::state_api::StateReader;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{DeprecatedTransactionInfo, TransactionInfo};
use mc_db::MadaraStorageRead;
use starknet_api::contract_class::EntryPointType;
use starknet_api::core::EntryPointSelector;
use starknet_api::transaction::fields::Calldata;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

impl<D: MadaraStorageRead> ExecutionContext<D> {
    /// Call a contract, returning the retdata.
    pub fn call_contract(
        &mut self,
        contract_address: &Felt,
        entry_point_selector: &Felt,
        calldata: &[Felt],
    ) -> Result<Vec<Felt>, Error> {
        tracing::debug!("calling contract {contract_address:#x}");

        // We don't need a tx_executor here
        let view = self.view().clone();
        let make_err = |err| CallContractError { view: format!("{view}"), contract: *contract_address, err };

        let storage_address =
            (*contract_address).try_into().map_err(TransactionExecutionError::StarknetApiError).map_err(make_err)?;
        let entry_point_selector = EntryPointSelector(*entry_point_selector);

        let entrypoint = CallEntryPoint {
            code_address: None,
            entry_point_type: EntryPointType::External,
            entry_point_selector,
            calldata: Calldata(Arc::new(calldata.to_vec())),
            storage_address,
            call_type: CallType::Call,
            initial_gas: self.block_context.versioned_constants().infinite_gas_for_vm_mode(),
            ..Default::default()
        };

        let mut entry_point_execution_context = EntryPointExecutionContext::new_invoke(
            Arc::new(TransactionContext {
                block_context: Arc::clone(&self.block_context),
                tx_info: TransactionInfo::Deprecated(DeprecatedTransactionInfo::default()),
            }),
            /* limit_steps_by_ressources */ false,
            SierraGasRevertTracker::new(entrypoint.initial_gas.into()),
        );

        let mut remaining_gas = entrypoint.initial_gas;

        let class_hash = self
            .state
            .get_class_hash_at(storage_address)
            .map_err(TransactionExecutionError::StateError)
            .map_err(make_err)?;

        let res = entrypoint
            .execute(&mut self.state, &mut entry_point_execution_context, &mut remaining_gas)
            .map_err(|error| TransactionExecutionError::ExecutionError {
                error: Box::new(error),
                class_hash,
                storage_address,
                selector: entry_point_selector,
            })
            .map_err(make_err)?;

        // When reverts are enabled (protocol >= 0.13.2), blockifier does not return an error for a
        // missing entrypoint or a contract panic: it returns a CallInfo with `execution.failed` set
        // and the failure reason in the retdata. Surface those as errors instead of a successful
        // call result.
        if res.execution.failed {
            let error = if res.execution.retdata.0 == [ENTRYPOINT_NOT_FOUND_ERROR_FELT] {
                EntryPointExecutionError::PreExecutionError(PreExecutionError::EntryPointNotFound(entry_point_selector))
            } else {
                EntryPointExecutionError::ExecutionFailed {
                    error_trace: extract_trailing_cairo1_revert_trace(&res, Cairo1RevertHeader::Execution),
                }
            };
            return Err(make_err(TransactionExecutionError::ExecutionError {
                error: Box::new(error),
                // NB: `class_hash` is `ClassHash(0)` if the contract is not deployed. That cannot
                // happen here: a missing contract fails inside `execute` with
                // `UninitializedStorageAddress` (a hard error, handled above), never with
                // `execution.failed`.
                class_hash,
                storage_address,
                selector: entry_point_selector,
            })
            .into());
        }

        Ok(res.execution.retdata.0)
    }
}
