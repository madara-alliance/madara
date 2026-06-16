use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_rpc::v0_9_0::{BlockId, FunctionCall};
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
    .await?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{rpc_test_setup_with_execution, TEST_CONTRACT_ADDRESS};
    use assert_matches::assert_matches;
    use mp_convert::ToFelt;
    use mp_rpc::v0_9_0::{BlockTag, FunctionCall};
    use starknet_core::utils::get_selector_from_name;
    use std::sync::Arc;

    /// Since blockifier 0.13.4, calling a non-existent entrypoint produces a *failed* execution
    /// with `ENTRYPOINT_NOT_FOUND` retdata instead of an error. This must surface as RPC error 21,
    /// not as a successful call result.
    #[tokio::test]
    async fn call_non_existent_entrypoint_returns_entrypoint_not_found() {
        let (backend, rpc, _keys) = rpc_test_setup_with_execution().await;

        let request = FunctionCall {
            contract_address: backend.chain_config().native_fee_token_address.to_felt(),
            entry_point_selector: get_selector_from_name("non_existent_entrypoint").unwrap(),
            calldata: Arc::new(vec![]),
        };
        let result = call(&rpc, request, BlockId::Tag(BlockTag::Latest)).await;

        assert_eq!(result.unwrap_err(), StarknetRpcApiError::EntrypointNotFound);
    }

    #[tokio::test]
    async fn call_non_existent_contract_returns_contract_not_found() {
        let (_backend, rpc, _keys) = rpc_test_setup_with_execution().await;

        let request = FunctionCall {
            contract_address: Felt::from_hex_unchecked("0xdeadbeefdeadbeef"),
            entry_point_selector: get_selector_from_name("non_existent_entrypoint").unwrap(),
            calldata: Arc::new(vec![]),
        };
        let result = call(&rpc, request, BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::ContractNotFound { .. });
    }

    /// A contract panic must surface as CONTRACT_ERROR with the failure reason as data, not as a
    /// successful call result containing the panic retdata.
    #[tokio::test]
    async fn call_reverted_returns_contract_error() {
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
            assert!(revert_error.contains("ERC20: transfer from 0"), "unexpected revert error: {revert_error}");
        });
    }

    #[tokio::test]
    async fn call_existing_entrypoint_succeeds() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let request = FunctionCall {
            contract_address: backend.chain_config().native_fee_token_address.to_felt(),
            entry_point_selector: get_selector_from_name("balance_of").unwrap(),
            calldata: Arc::new(vec![keys.0[0].address]),
        };
        let result = call(&rpc, request, BlockId::Tag(BlockTag::Latest)).await.unwrap();

        // u256 balance: [low, high], non-zero for a funded devnet account.
        assert_eq!(result.len(), 2);
        assert_ne!(result[0], Felt::ZERO);
    }

    /// Calling the test contract's l1 handler through starknet_call must not work as an external
    /// entrypoint, but the selector exists on the contract: sanity-check that we still classify
    /// this as entrypoint-not-found (it is not an *external* entrypoint).
    #[tokio::test]
    async fn call_l1_handler_selector_returns_entrypoint_not_found() {
        let (_backend, rpc, _keys) = rpc_test_setup_with_execution().await;

        let request = FunctionCall {
            contract_address: TEST_CONTRACT_ADDRESS,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            calldata: Arc::new(vec![Felt::ONE, Felt::TWO, Felt::THREE]),
        };
        let result = call(&rpc, request, BlockId::Tag(BlockTag::Latest)).await;

        assert_eq!(result.unwrap_err(), StarknetRpcApiError::EntrypointNotFound);
    }
}
