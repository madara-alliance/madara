use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::versions::user::v0_7_1::methods::read::call::call_with;
use crate::Starknet;
use mp_rpc::v0_8_1::{BlockId, FunctionCall};
use starknet_types_core::felt::Felt;

/// Call a Function in a Contract Without Creating a Transaction
///
/// Same implementation as v0.7.1, but starting with v0.8 the spec structures execution failures:
/// CONTRACT_ERROR `revert_error` data is the nested CONTRACT_EXECUTION_ERROR object instead of a
/// flat string (matching pathfinder, which switches representations at v0.8).
pub async fn call(starknet: &Starknet, request: FunctionCall, block_id: BlockId) -> StarknetRpcResult<Vec<Felt>> {
    call_with(starknet, request, block_id, StarknetRpcApiError::from).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::rpc_test_setup_with_execution;
    use assert_matches::assert_matches;
    use mp_convert::ToFelt;
    use mp_rpc::v0_8_1::BlockTag;
    use starknet_core::utils::get_selector_from_name;
    use std::sync::Arc;

    /// The v0.8.1 endpoint must return the structured CONTRACT_EXECUTION_ERROR object, not the
    /// v0.7.1 flat string it previously inherited through delegation.
    #[tokio::test]
    async fn call_reverted_returns_structured_revert_error() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let fee_token = backend.chain_config().native_fee_token_address.to_felt();
        // The caller address of starknet_call is 0: the ERC20 panics with
        // 'ERC20: transfer from 0'.
        let request = FunctionCall {
            contract_address: fee_token,
            entry_point_selector: get_selector_from_name("transfer").unwrap(),
            calldata: Arc::new(vec![keys.0[0].address, Felt::ONE, Felt::ZERO]),
        };
        let result = call(&rpc, request, BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::ContractError { revert_error } => {
            assert!(revert_error.is_object(), "v0.8+ revert_error must be structured, got: {revert_error}");
            assert_eq!(revert_error["contract_address"], serde_json::json!(fee_token));
        });
    }
}
