use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

/// Get the contract class hash in the given block for the contract deployed at the given
/// address
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag
/// * `contract_address` - The address of the contract whose class hash will be returned
///
/// ### Returns
///
/// * `class_hash` - The class hash of the given contract
pub fn get_class_hash_at(starknet: &Starknet, block_id: BlockId, contract_address: Felt) -> StarknetRpcResult<Felt> {
    // Check if block exists. We have to return a different error in that case.
    let block_exists =
        starknet.backend.contains_block(&block_id).or_internal_server_error("Checking if block is in database")?;
    if !block_exists {
        return Err(StarknetRpcApiError::BlockNotFound);
    }

    let class_hash = starknet
        .backend
        .get_contract_class_hash_at(&block_id, &contract_address)
        .or_internal_server_error("Error getting contract class hash at")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    Ok(class_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use rstest::rstest;
    use starknet_core::types::BlockTag;

    #[rstest]
    fn test_get_class_hash_at(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, class_hashes, .. }, rpc) = sample_chain_for_state_updates;

        // Block 0
        let block_n = BlockId::Number(0);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[0]).unwrap(), class_hashes[0]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[1]), Err(StarknetRpcApiError::ContractNotFound));
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]), Err(StarknetRpcApiError::ContractNotFound));

        // Block 1
        let block_n = BlockId::Number(1);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[0]).unwrap(), class_hashes[0]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[1]).unwrap(), class_hashes[1]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]).unwrap(), class_hashes[0]);

        // Block 2
        let block_n = BlockId::Number(2);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[0]).unwrap(), class_hashes[0]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[1]).unwrap(), class_hashes[1]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]).unwrap(), class_hashes[0]);

        // Pending
        let block_n = BlockId::Tag(BlockTag::Pending);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[0]).unwrap(), class_hashes[2]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[1]).unwrap(), class_hashes[1]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]).unwrap(), class_hashes[0]);
    }

    #[rstest]
    fn test_get_class_hash_at_not_found(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, .. }, rpc) = sample_chain_for_state_updates;

        // Not found
        let block_n = BlockId::Number(3);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[0]), Err(StarknetRpcApiError::BlockNotFound));
        let block_n = BlockId::Number(0);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[1]), Err(StarknetRpcApiError::ContractNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_class_hash_at(&rpc, block_n, does_not_exist), Err(StarknetRpcApiError::ContractNotFound));
    }
}
