use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_10_0::{BlockId, MaybeDeprecatedContractClass};
use starknet_types_core::felt::Felt;

pub fn get_class(
    starknet: &Starknet,
    block_id: BlockId,
    class_hash: Felt,
) -> StarknetRpcResult<MaybeDeprecatedContractClass> {
    let view = starknet.resolve_view_on(block_id)?;
    let class_info = view.get_class_info(&class_hash)?.ok_or(StarknetRpcApiError::class_hash_not_found())?;

    Ok(class_info.contract_class().into())
}
