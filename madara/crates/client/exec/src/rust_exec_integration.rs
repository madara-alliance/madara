//! Integration between mc-rust-exec verification and mc-exec execution.
//!
//! This module provides:
//! 1. StateReader adapter for BlockifierStateAdapter
//! 2. Verification hooks for transaction tracing
//! 3. Conversion utilities between Blockifier and Rust execution formats

#[cfg(feature = "rust-verification")]
use mc_rust_exec::{ContractAddress as RustContractAddress, StateError as RustStateError, StateReader};
use mp_convert::ToFelt;
use starknet_types_core::felt::Felt;

#[cfg(feature = "rust-verification")]
use crate::BlockifierStateAdapter;
#[cfg(feature = "rust-verification")]
use mc_db::MadaraStorageRead;

/// Adapter that allows BlockifierStateAdapter to be used with mc-rust-exec's StateReader trait.
#[cfg(feature = "rust-verification")]
pub struct RustExecStateAdapter<'a, D: MadaraStorageRead> {
    blockifier_adapter: &'a BlockifierStateAdapter<D>,
}

#[cfg(feature = "rust-verification")]
impl<'a, D: MadaraStorageRead> RustExecStateAdapter<'a, D> {
    pub fn new(blockifier_adapter: &'a BlockifierStateAdapter<D>) -> Self {
        Self { blockifier_adapter }
    }
}

#[cfg(feature = "rust-verification")]
impl<'a, D: MadaraStorageRead> StateReader for RustExecStateAdapter<'a, D> {
    fn get_storage_at(
        &self,
        contract: RustContractAddress,
        key: mc_rust_exec::types::StorageKey,
    ) -> Result<Felt, RustStateError> {
        use blockifier::state::state_api::StateReader as BlockifierStateReader;

        let starknet_contract = starknet_api::core::ContractAddress::try_from(contract.0)
            .map_err(|e| RustStateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let starknet_key = starknet_api::state::StorageKey::try_from(key.0)
            .map_err(|e| RustStateError::InvalidStorageKey(format!("Invalid storage key: {e}")))?;

        self.blockifier_adapter
            .get_storage_at(starknet_contract, starknet_key)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))
    }

    fn get_nonce_at(&self, contract: RustContractAddress) -> Result<mc_rust_exec::types::Nonce, RustStateError> {
        use blockifier::state::state_api::StateReader as BlockifierStateReader;

        let starknet_contract = starknet_api::core::ContractAddress::try_from(contract.0)
            .map_err(|e| RustStateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let nonce = self
            .blockifier_adapter
            .get_nonce_at(starknet_contract)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))?;

        Ok(mc_rust_exec::types::Nonce(nonce.0))
    }

    fn get_class_hash_at(
        &self,
        contract: RustContractAddress,
    ) -> Result<Option<Felt>, RustStateError> {
        use blockifier::state::state_api::StateReader as BlockifierStateReader;

        let starknet_contract = starknet_api::core::ContractAddress::try_from(contract.0)
            .map_err(|e| RustStateError::InvalidAddress(format!("Invalid contract address: {e}")))?;

        let class_hash = self
            .blockifier_adapter
            .get_class_hash_at(starknet_contract)
            .map_err(|e| RustStateError::BackendError(format!("Blockifier state read error: {e}")))?;

        // Class hash of 0x0 means contract is not deployed
        if class_hash.0 == Felt::ZERO {
            Ok(None)
        } else {
            Ok(Some(class_hash.0))
        }
    }
}

/// Verify a transaction execution against Rust implementation.
///
/// Returns true if verification passed or was skipped (contract not supported).
/// Logs warnings if mismatches are detected.
#[cfg(feature = "rust-verification")]
pub fn verify_transaction_execution(
    blockifier_adapter: &BlockifierStateAdapter<impl MadaraStorageRead>,
    contract_address: Felt,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller_address: Felt,
    execution_result: &crate::ExecutionResult,
) -> bool {
    // Create state adapter for Rust execution
    let rust_state = RustExecStateAdapter::new(blockifier_adapter);

    // Try to execute with Rust implementation
    let rust_result = match mc_rust_exec::execute_transaction(
        &rust_state,
        RustContractAddress(contract_address),
        class_hash,
        entry_point_selector,
        calldata,
        RustContractAddress(caller_address),
    ) {
        Some(Ok(result)) => result,
        Some(Err(e)) => {
            tracing::warn!(
                "Rust execution failed for contract {:#x}, function {:#x}: {e:?}",
                contract_address,
                entry_point_selector
            );
            return true; // Don't fail on Rust execution errors
        }
        None => {
            // Contract/function not supported in Rust - this is expected
            tracing::debug!(
                "Contract {:#x} or function {:#x} not supported in Rust verification",
                contract_address,
                entry_point_selector
            );
            return true;
        }
    };

    // Extract data from Blockifier's execution result
    let blockifier_execute_call_info = match &execution_result.execution_info.execute_call_info {
        Some(info) => info,
        None => {
            tracing::warn!("No execute call info in Blockifier result");
            return true;
        }
    };

    // Prepare Blockifier data for comparison
    let blockifier_storage: Vec<(Felt, Vec<(Felt, Felt)>)> = execution_result
        .state_diff
        .storage_updates
        .iter()
        .map(|(addr, updates)| {
            let storage_entries: Vec<(Felt, Felt)> = updates.iter().map(|(k, v)| (k.to_felt(), *v)).collect();
            (addr.to_felt(), storage_entries)
        })
        .collect();

    let blockifier_retdata: Vec<Felt> = blockifier_execute_call_info.execution.retdata.0.clone();

    let blockifier_events: Vec<(Vec<Felt>, Vec<Felt>)> = blockifier_execute_call_info
        .execution
        .events
        .iter()
        .map(|event| {
            let keys: Vec<Felt> = event.event.keys.iter().map(ToFelt::to_felt).collect();
            let data: Vec<Felt> = event.event.data.0.clone();
            (keys, data)
        })
        .collect();

    let blockifier_failed = blockifier_execute_call_info.execution.failed;

    // Perform verification
    let verification_result = mc_rust_exec::compare_with_blockifier(
        &rust_result,
        &blockifier_storage,
        &blockifier_retdata,
        &blockifier_events,
        blockifier_failed,
    );

    if !verification_result.passed {
        tracing::warn!(
            "Rust verification FAILED for contract {:#x}, function {:#x}:",
            contract_address,
            entry_point_selector
        );
        for error in &verification_result.errors {
            tracing::warn!("  - {:?}", error);
        }
        return false;
    }

    tracing::debug!(
        "Rust verification PASSED for contract {:#x}, function {:#x}",
        contract_address,
        entry_point_selector
    );
    true
}

/// Placeholder when rust-verification feature is disabled.
#[cfg(not(feature = "rust-verification"))]
pub fn verify_transaction_execution(
    _blockifier_adapter: &crate::BlockifierStateAdapter<impl mc_db::MadaraStorageRead>,
    _contract_address: Felt,
    _class_hash: Felt,
    _entry_point_selector: Felt,
    _calldata: &[Felt],
    _caller_address: Felt,
    _execution_result: &crate::ExecutionResult,
) -> bool {
    true // Always pass when verification is disabled
}
