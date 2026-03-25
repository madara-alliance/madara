use crate::errors::StarknetRpcResult;
use crate::versions::user::v0_9_0::methods::read::get_messages_status as v0_9_0;
use crate::Starknet;
use mp_rpc::v0_10_2::{L1TxnHash, MessageStatus};

pub fn get_messages_status(starknet: &Starknet, transaction_hash: L1TxnHash) -> StarknetRpcResult<Vec<MessageStatus>> {
    v0_9_0::get_messages_status(starknet, transaction_hash)
}
