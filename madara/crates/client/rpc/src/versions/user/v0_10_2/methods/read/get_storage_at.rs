use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{GetStorageAtResult, StorageResponseFlag, StorageResult};
use starknet_types_core::felt::Felt;

pub fn get_storage_at(
    starknet: &Starknet,
    contract_address: Felt,
    key: Felt,
    block_id: BlockId,
    response_flags: Option<Vec<StorageResponseFlag>>,
) -> StarknetRpcResult<GetStorageAtResult> {
    let view = starknet.resolve_view_on(block_id)?;

    // Felt::ONE and Felt::TWO are internal system mappings, so they don't require a deployed contract.
    if contract_address != Felt::ONE
        && contract_address != Felt::TWO
        && !view.is_contract_deployed(&contract_address)?
    {
        return Err(StarknetRpcApiError::contract_not_found());
    }

    let value = view.get_contract_storage(&contract_address, &key)?.unwrap_or(Felt::ZERO);
    let include_last_update_block =
        response_flags.as_ref().is_some_and(|flags| flags.contains(&StorageResponseFlag::IncludeLastUpdateBlock));

    if include_last_update_block {
        Ok(GetStorageAtResult::Result(StorageResult {
            value,
            last_update_block: find_last_update_block(&view, contract_address, key)?,
        }))
    } else {
        Ok(GetStorageAtResult::Value(value))
    }
}

// TODO (mohit 08/04/2026): This performs a backward scan over state diffs and can add
// significant load to getStorageAt when INCLUDE_LAST_UPDATE_BLOCK is requested.
// Replace this with indexed or cached last-write tracking instead of recomputing it per request.
fn find_last_update_block(
    view: &mc_db::MadaraStateView,
    contract_address: Felt,
    key: Felt,
) -> Result<u64, StarknetRpcApiError> {
    let storage_updated = |state_diff: &mp_state_update::StateDiff| {
        state_diff
            .storage_diffs
            .iter()
            .any(|diff| diff.address == contract_address && diff.storage_entries.iter().any(|entry| entry.key == key))
    };

    if let Some(preconfirmed) = view.block_view_on_latest().and_then(|block| block.as_preconfirmed()) {
        let state_diff = preconfirmed
            .get_normalized_state_diff()
            .map_err(|err| StarknetRpcApiError::ErrUnexpectedError { error: err.to_string().into() })?;
        if storage_updated(&state_diff) {
            return Ok(preconfirmed.block_number());
        }
    }

    let Some(latest_confirmed_block_n) = view.latest_confirmed_block_n() else {
        return Ok(0);
    };

    for block_number in (0..=latest_confirmed_block_n).rev() {
        let Some(block_view) = view.block_view_on_confirmed(block_number) else {
            continue;
        };
        let state_diff = block_view
            .get_state_diff()
            .map_err(|err| StarknetRpcApiError::ErrUnexpectedError { error: err.to_string().into() })?;
        if storage_updated(&state_diff) {
            return Ok(block_number);
        }
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use mp_rpc::v0_10_0::BlockTag;
    use rstest::rstest;

    #[rstest]
    fn test_get_storage_at_value(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, keys, values, .. }, rpc) = sample_chain_for_state_updates;

        let result = get_storage_at(&rpc, contracts[0], keys[0], BlockId::Number(2), None).unwrap();
        assert_eq!(result, GetStorageAtResult::Value(values[1]));
    }

    #[rstest]
    fn test_get_storage_at_with_last_update_block(
        sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet),
    ) {
        let (SampleChainForStateUpdates { contracts, keys, values, .. }, rpc) = sample_chain_for_state_updates;

        let result = get_storage_at(
            &rpc,
            contracts[0],
            keys[0],
            BlockId::Number(2),
            Some(vec![StorageResponseFlag::IncludeLastUpdateBlock]),
        )
        .unwrap();
        assert_eq!(result, GetStorageAtResult::Result(StorageResult { value: values[1], last_update_block: 1 }));

        let pending_result = get_storage_at(
            &rpc,
            contracts[0],
            keys[1],
            BlockId::Tag(BlockTag::PreConfirmed),
            Some(vec![StorageResponseFlag::IncludeLastUpdateBlock]),
        )
        .unwrap();
        assert_eq!(
            pending_result,
            GetStorageAtResult::Result(StorageResult { value: values[0], last_update_block: 3 })
        );
    }

    #[rstest]
    fn test_get_storage_at_last_update_block_zero_for_unset_key(
        sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet),
    ) {
        let (SampleChainForStateUpdates { contracts, keys, .. }, rpc) = sample_chain_for_state_updates;

        let result = get_storage_at(
            &rpc,
            contracts[1],
            keys[1],
            BlockId::Number(2),
            Some(vec![StorageResponseFlag::IncludeLastUpdateBlock]),
        )
        .unwrap();
        assert_eq!(result, GetStorageAtResult::Result(StorageResult { value: Felt::ZERO, last_update_block: 0 }));
    }
}
