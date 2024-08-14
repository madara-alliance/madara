use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

/// Get the nonce associated with the given address in the given block.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter specifies the block in which the nonce is to be checked.
/// * `contract_address` - The address of the contract whose nonce we're seeking. This is the unique
///   identifier of the contract in the Starknet network.
///
/// ### Returns
///
/// Returns the contract's nonce at the requested state. The nonce is returned as a
/// `Felt`, representing the current state of the contract in terms of transactions
/// count or other contract-specific operations. In case of errors, such as
/// `BLOCK_NOT_FOUND` or `CONTRACT_NOT_FOUND`, returns a `StarknetRpcApiError` indicating the
/// specific issue.

pub fn get_nonce(starknet: &Starknet, block_id: BlockId, contract_address: Felt) -> StarknetRpcResult<Felt> {
    // Check if block exists. We have to return a different error in that case.
    let block_exists =
        starknet.backend.contains_block(&block_id).or_internal_server_error("Checking if block is in database")?;
    if !block_exists {
        return Err(StarknetRpcApiError::BlockNotFound);
    }

    if !starknet
        .backend
        .is_contract_deployed_at(&block_id, &contract_address)
        .or_internal_server_error("Error checking if contract exists")?
    {
        return Err(StarknetRpcApiError::ContractNotFound);
    }

    let nonce = starknet
        .backend
        .get_contract_nonce_at(&block_id, &contract_address)
        .or_internal_server_error("Error getting nonce")?
        .unwrap_or(Felt::ZERO);

    Ok(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use rstest::rstest;
    use starknet_core::types::BlockTag;

    #[rstest]
    fn test_get_nonce(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, .. }, rpc) = sample_chain_for_state_updates;

        // Block 0
        let block_n = BlockId::Number(0);
        assert_eq!(get_nonce(&rpc, block_n, contracts[0]).unwrap(), 0.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[1]), Err(StarknetRpcApiError::ContractNotFound));
        assert_eq!(get_nonce(&rpc, block_n, contracts[2]), Err(StarknetRpcApiError::ContractNotFound));

        // Block 1
        let block_n = BlockId::Number(1);
        assert_eq!(get_nonce(&rpc, block_n, contracts[0]).unwrap(), 1.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[1]).unwrap(), 0.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[2]).unwrap(), 2.into());

        // Block 2
        let block_n = BlockId::Number(2);
        assert_eq!(get_nonce(&rpc, block_n, contracts[0]).unwrap(), 1.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[1]).unwrap(), 0.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[2]).unwrap(), 2.into());

        // Pending
        let block_n = BlockId::Tag(BlockTag::Pending);
        assert_eq!(get_nonce(&rpc, block_n, contracts[0]).unwrap(), 3.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[1]).unwrap(), 2.into());
        assert_eq!(get_nonce(&rpc, block_n, contracts[2]).unwrap(), 2.into());
    }

    #[rstest]
    fn test_get_nonce_not_found(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, .. }, rpc) = sample_chain_for_state_updates;

        // Not found
        let block_n = BlockId::Number(3);
        assert_eq!(get_nonce(&rpc, block_n, contracts[0]), Err(StarknetRpcApiError::BlockNotFound));
        let block_n = BlockId::Number(0);
        assert_eq!(get_nonce(&rpc, block_n, contracts[1]), Err(StarknetRpcApiError::ContractNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_nonce(&rpc, block_n, does_not_exist), Err(StarknetRpcApiError::ContractNotFound));
    }
}
