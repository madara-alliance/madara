use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_rpc::v0_7_1::{BlockId, FunctionCall};
use starknet_types_core::felt::Felt;

/// Call a Function in a Contract Without Creating a Transaction
///
/// ### Arguments
///
/// * `request` - The details of the function call to be made. This includes information such as the
///   contract address, function signature, and arguments.
/// * `block_id` - The identifier of the block used to reference the state or call the transaction
///   on. This can be the hash of the block, its number (height), or a specific block tag.
///
/// ### Returns
///
/// * `result` - The function's return value, as defined in the Cairo output. This is an array of
///   field elements (`Felt`).
///
/// ### Errors
///
/// This method may return the following errors:
/// * `CONTRACT_NOT_FOUND` - If the specified contract address does not exist.
/// * `CONTRACT_ERROR` - If there is an error with the contract or the function call.
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
pub async fn call(starknet: &Starknet, request: FunctionCall, block_id: BlockId) -> StarknetRpcResult<Vec<Felt>> {
    let view = starknet.resolve_block_view(block_id)?;

    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let FunctionCall { contract_address, entry_point_selector, calldata } = request;
    // spawn_blocking: avoid starving the tokio workers during execution.
    let results = mp_utils::spawn_blocking(move || {
        exec_context.call_contract(&contract_address, &entry_point_selector, &calldata)
    })
    .await
    .map_err(StarknetRpcApiError::from_exec_error_v0_7)?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::rpc_test_setup_with_execution;
    use assert_matches::assert_matches;
    use mp_convert::ToFelt;
    use mp_rpc::v0_7_1::BlockTag;
    use starknet_core::utils::get_selector_from_name;
    use std::sync::Arc;

    /// v0.7.1 predates structured execution errors: CONTRACT_ERROR `revert_error` data must be a
    /// flat string, not the structured object used by v0.8+.
    #[tokio::test]
    async fn call_reverted_returns_string_revert_error() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        // The caller address of starknet_call is 0: the ERC20 panics with
        // 'ERC20: transfer from 0'.
        let request = FunctionCall {
            contract_address: backend.chain_config().native_fee_token_address.to_felt(),
            entry_point_selector: get_selector_from_name("transfer").unwrap(),
            calldata: Arc::new(vec![keys.0[0].address, Felt::ONE, Felt::ZERO]),
        };
        let result = call(&rpc, request, BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::ContractError { revert_error } => {
            assert!(revert_error.is_string(), "v0.7.1 revert_error must be a flat string, got: {revert_error}");
            assert!(
                revert_error.as_str().unwrap().contains("ERC20: transfer from 0"),
                "unexpected revert error: {revert_error}"
            );
        });
    }
}
