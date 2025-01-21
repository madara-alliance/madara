use crate::utils::ResultExt;
use crate::versions::user::{
    v0_7_1::methods::read::get_transaction_status::get_transaction_status, v0_8_0::MessageStatus,
};
use crate::{Starknet, StarknetRpcApiError, StarknetRpcResult};
use alloy::primitives::TxHash;

pub fn get_messages_status(starknet: &Starknet, transaction_hash: TxHash) -> StarknetRpcResult<Vec<MessageStatus>> {
    let l1_handler_tx_hashes = starknet
        .backend
        .get_l1_handler_tx_hashes(transaction_hash)
        .or_internal_server_error("Retrieving L1 handler transactions from database")?;
    if l1_handler_tx_hashes.is_empty() {
        return Err(StarknetRpcApiError::TxnHashNotFound);
    }
    l1_handler_tx_hashes.iter().try_fold(
        Vec::with_capacity(l1_handler_tx_hashes.len()),
        |mut acc, l1_handler_tx_hash| {
            let finality_status = match get_transaction_status(starknet, *l1_handler_tx_hash) {
                Ok(tx_status) => tx_status.finality_status,
                Err(StarknetRpcApiError::TxnHashNotFound) => {
                    tracing::error!("L1 handler tx {l1_handler_tx_hash:?} for L1 tx {transaction_hash:?} not found");
                    return Err(StarknetRpcApiError::InternalServerError);
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to retrieve transaction status for L1 handler transaction {l1_handler_tx_hash:?} \
                         related to L1 transaction {transaction_hash:?}: {e:?}"
                    );
                    return Err(StarknetRpcApiError::InternalServerError);
                }
            };
            acc.push(MessageStatus {
                transaction_hash: *l1_handler_tx_hash,
                finality_status,
                // TODO Update this once get_transaction_status supports rejections
                failure_reason: None,
            });
            Ok(acc)
        },
    )
}
