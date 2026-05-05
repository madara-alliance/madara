use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{MaybePreConfirmedStateUpdate, PreConfirmedStateUpdate, StateUpdate};
use mp_state_update::StateDiff;
use starknet_types_core::felt::Felt;
use std::collections::HashSet;

pub fn get_state_update(
    starknet: &Starknet,
    block_id: BlockId,
    contract_addresses: Option<Vec<Felt>>,
) -> StarknetRpcResult<MaybePreConfirmedStateUpdate> {
    let view = starknet.resolve_block_view(block_id)?;
    let mut state_diff = view.get_state_diff()?;
    let filter = contract_addresses.unwrap_or_default();
    if !filter.is_empty() {
        filter_state_diff(&mut state_diff, &HashSet::from_iter(filter));
    }

    let old_root = if let Some(parent) = view.parent_block() {
        parent.get_block_info()?.header.global_state_root
    } else {
        Felt::ZERO
    };

    if let Some(confirmed) = view.as_confirmed() {
        let block_info = confirmed.get_block_info()?;
        Ok(MaybePreConfirmedStateUpdate::Block(StateUpdate {
            block_hash: block_info.block_hash,
            old_root,
            new_root: block_info.header.global_state_root,
            state_diff: state_diff.into(),
        }))
    } else {
        Ok(MaybePreConfirmedStateUpdate::PreConfirmed(PreConfirmedStateUpdate { state_diff: state_diff.into() }))
    }
}

fn filter_state_diff(state_diff: &mut StateDiff, contract_addresses: &HashSet<Felt>) {
    state_diff.storage_diffs.retain(|diff| contract_addresses.contains(&diff.address));
    state_diff.deployed_contracts.retain(|contract| contract_addresses.contains(&contract.address));
    state_diff.replaced_classes.retain(|contract| contract_addresses.contains(&contract.contract_address));
    state_diff.nonces.retain(|nonce| contract_addresses.contains(&nonce.contract_address));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use mp_rpc::v0_10_0::BlockTag;
    use rstest::rstest;

    #[rstest]
    fn test_get_state_update_without_filter(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { state_diffs, state_roots, block_hashes, .. }, rpc) =
            sample_chain_for_state_updates;

        assert_eq!(
            get_state_update(&rpc, BlockId::Number(2), None).unwrap(),
            MaybePreConfirmedStateUpdate::Block(StateUpdate {
                block_hash: block_hashes[2],
                old_root: state_roots[1],
                new_root: state_roots[2],
                state_diff: state_diffs[2].clone().into(),
            })
        );
    }

    #[rstest]
    fn test_get_state_update_with_contract_filter(
        sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet),
    ) {
        let (SampleChainForStateUpdates { contracts, state_roots, block_hashes, .. }, rpc) =
            sample_chain_for_state_updates;

        let result = get_state_update(&rpc, BlockId::Number(1), Some(vec![contracts[2]])).unwrap();
        let MaybePreConfirmedStateUpdate::Block(result) = result else {
            panic!("expected confirmed state update");
        };

        assert_eq!(result.block_hash, block_hashes[1]);
        assert_eq!(result.old_root, state_roots[0]);
        assert_eq!(result.new_root, state_roots[1]);
        assert_eq!(result.state_diff.declared_classes.len(), 0);
        assert_eq!(result.state_diff.deprecated_declared_classes.len(), 0);
        assert_eq!(result.state_diff.migrated_compiled_classes.len(), 0);
        assert_eq!(result.state_diff.deployed_contracts.len(), 1);
        assert_eq!(result.state_diff.deployed_contracts[0].address, contracts[2]);
        assert_eq!(result.state_diff.storage_diffs.len(), 1);
        assert_eq!(result.state_diff.storage_diffs[0].address, contracts[2]);
        assert_eq!(result.state_diff.nonces.len(), 1);
        assert_eq!(result.state_diff.nonces[0].contract_address, contracts[2]);
    }

    #[rstest]
    fn test_get_state_update_keeps_declarations_when_filtered(
        sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet),
    ) {
        let (SampleChainForStateUpdates { contracts, .. }, rpc) = sample_chain_for_state_updates;

        let result = get_state_update(&rpc, BlockId::Tag(BlockTag::PreConfirmed), Some(vec![contracts[1]])).unwrap();
        let MaybePreConfirmedStateUpdate::PreConfirmed(result) = result else {
            panic!("expected preconfirmed state update");
        };

        assert_eq!(result.state_diff.storage_diffs.len(), 0);
        assert_eq!(result.state_diff.replaced_classes.len(), 0);
        assert_eq!(result.state_diff.nonces.len(), 1);
        assert_eq!(result.state_diff.nonces[0].contract_address, contracts[1]);
        assert_eq!(result.state_diff.declared_classes.len(), 1);
    }
}
