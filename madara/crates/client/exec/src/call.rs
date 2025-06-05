use std::sync::Arc;

use blockifier::context::TransactionContext;
use blockifier::execution::entry_point::{
    CallEntryPoint, CallType, EntryPointExecutionContext, SierraGasRevertTracker,
};
use blockifier::state::state_api::StateReader;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{DeprecatedTransactionInfo, TransactionInfo};
use starknet_api::contract_class::EntryPointType;
use starknet_api::core::EntryPointSelector;
use starknet_api::transaction::fields::Calldata;
use starknet_types_core::felt::Felt;

use crate::{CallContractError, Error, ExecutionContext};

impl ExecutionContext {
    /// Call a contract, returning the retdata.
    pub fn call_contract(
        &self,
        contract_address: &Felt,
        entry_point_selector: &Felt,
        calldata: &[Felt],
    ) -> Result<Vec<Felt>, Error> {
        tracing::debug!("calling contract {contract_address:#x}");

        // We don't need a tx_executor here

        let make_err =
            |err| CallContractError { block_n: self.latest_visible_block.into(), contract: *contract_address, err };

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
            false,
            SierraGasRevertTracker::new(entrypoint.initial_gas.into()),
        );

        let mut cached_state = self.init_cached_state();

        let mut remaining_gas = entrypoint.initial_gas;

        let class_hash = cached_state
            .get_class_hash_at(storage_address)
            .map_err(TransactionExecutionError::StateError)
            .map_err(make_err)?;

        let res = entrypoint
            .execute(&mut cached_state, &mut entry_point_execution_context, &mut remaining_gas)
            .map_err(|error| TransactionExecutionError::ExecutionError {
                error,
                class_hash,
                storage_address,
                selector: entry_point_selector,
            })
            .map_err(make_err)?;

        Ok(res.execution.retdata.0)
    }
}
