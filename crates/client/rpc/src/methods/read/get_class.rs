use dc_db::storage_handler::StorageView;
use starknet_core::types::{BlockId, ContractClass, Felt};

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

pub fn get_class(starknet: &Starknet, block_id: BlockId, class_hash: Felt) -> StarknetRpcResult<ContractClass> {
    // Check if the given block exists
    starknet.get_block_info(block_id)?;

    let class = starknet
        .backend
        .contract_class_data()
        .get(&class_hash)
        .or_internal_server_error("Failed to retrieve contract class")?
        .ok_or(StarknetRpcApiError::ClassHashNotFound)?;

    Ok(class.contract_class.into())
}
