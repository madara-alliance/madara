use mp_block::BlockId;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::MaybeDeprecatedContractClass;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

pub fn get_class(
    starknet: &Starknet,
    block_id: BlockId,
    class_hash: Felt,
) -> StarknetRpcResult<MaybeDeprecatedContractClass<Felt>> {
    let class_data = starknet
        .backend
        .get_class_info(&block_id, &class_hash)
        .or_internal_server_error("Error getting contract class info")?
        .ok_or(StarknetRpcApiError::ClassHashNotFound)?;

    Ok(class_data.contract_class().into())
}
