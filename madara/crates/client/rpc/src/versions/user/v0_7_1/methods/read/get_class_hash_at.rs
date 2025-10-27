use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_7_1::BlockId;
use starknet_types_core::felt::Felt;

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
    let view = starknet.resolve_view_on(block_id)?;
    tracing::debug!("{view:?}");
    view.get_contract_class_hash(&contract_address)?.ok_or(StarknetRpcApiError::contract_not_found())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use mp_rpc::v0_7_1::BlockTag;
    use rstest::rstest;

    #[rstest]
    fn test_get_class_hash_at(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, class_hashes, .. }, rpc) = sample_chain_for_state_updates;

        // Block 0
        let block_n = BlockId::Number(0);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[0]).unwrap(), class_hashes[0]);
        assert_eq!(
            get_class_hash_at(&rpc, block_n.clone(), contracts[1]),
            Err(StarknetRpcApiError::contract_not_found())
        );
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]), Err(StarknetRpcApiError::contract_not_found()));

        // Block 1
        let block_n = BlockId::Number(1);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[0]).unwrap(), class_hashes[0]);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[1]).unwrap(), class_hashes[1]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]).unwrap(), class_hashes[0]);

        // Block 2
        let block_n = BlockId::Number(2);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[0]).unwrap(), class_hashes[0]);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[1]).unwrap(), class_hashes[1]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]).unwrap(), class_hashes[0]);

        // Pending
        let block_n = BlockId::Tag(BlockTag::Pending);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[0]).unwrap(), class_hashes[2]);
        assert_eq!(get_class_hash_at(&rpc, block_n.clone(), contracts[1]).unwrap(), class_hashes[1]);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[2]).unwrap(), class_hashes[0]);
    }

    #[rstest]
    fn test_get_class_hash_at_not_found(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { contracts, .. }, rpc) = sample_chain_for_state_updates;

        // Not found
        let block_n = BlockId::Number(3);
        assert_eq!(get_class_hash_at(&rpc, block_n, contracts[0]), Err(StarknetRpcApiError::BlockNotFound));
        let block_n = BlockId::Number(0);
        assert_eq!(
            get_class_hash_at(&rpc, block_n.clone(), contracts[1]),
            Err(StarknetRpcApiError::contract_not_found())
        );
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_class_hash_at(&rpc, block_n, does_not_exist), Err(StarknetRpcApiError::contract_not_found()));
    }
}
