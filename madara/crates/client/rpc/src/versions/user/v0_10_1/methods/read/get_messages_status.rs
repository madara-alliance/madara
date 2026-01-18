use crate::errors::StarknetRpcResult;
use crate::{Starknet, StarknetRpcApiError};
use mp_rpc::v0_10_1::{L1TxnHash, MessageStatus};

pub fn get_messages_status(_starknet: &Starknet, _transaction_hash: L1TxnHash) -> StarknetRpcResult<Vec<MessageStatus>> {
    // TODO: Implement via settlement layer provider once L1 sync data is available.
    Err(StarknetRpcApiError::TxnHashNotFound)
}
