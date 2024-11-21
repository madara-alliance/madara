use crate::utils::ResultExt;
use crate::versions::user::v0_7_1::methods::read::get_transaction_status::get_transaction_status;
use crate::{Starknet, StarknetRpcApiError, StarknetRpcResult};
use alloy::primitives::TxHash;
use jsonrpsee::core::Serialize;
use serde::Deserialize;
use starknet_core::types::SequencerTransactionStatus;
use starknet_types_core::felt::Felt;

pub fn get_messages_status(starknet: &Starknet, transaction_hash: TxHash) -> StarknetRpcResult<Vec<MessageStatus>> {
    let l1_handler_tx_hashes = starknet
        .backend
        .get_l1_handler_tx_hashes(transaction_hash)
        .or_internal_server_error("Retrieving L1 handler transactions from database")?;
    if l1_handler_tx_hashes.is_empty() {
        return Err(StarknetRpcApiError::TxnHashNotFound);
    }
    let mut message_statuses = vec![];
    for l1_handler_tx_hash in l1_handler_tx_hashes {
        let finality_status = match get_transaction_status(starknet, l1_handler_tx_hash) {
            Ok(tx_status) => tx_status.finality_status(),
            Err(StarknetRpcApiError::TxnHashNotFound) => {
                tracing::error!("L1 handler tx {l1_handler_tx_hash:?} for L1 tx {transaction_hash:?} not found");
                return Err(StarknetRpcApiError::InternalServerError);
            }
            Err(e) => return Err(e),
        };
        message_statuses.push(MessageStatus {
            transaction_hash: l1_handler_tx_hash,
            finality_status,
            // TODO Update this once get_transaction_status supports rejections
            failure_reason: None,
        })
    }
    Ok(message_statuses)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageStatus {
    pub transaction_hash: Felt,
    pub finality_status: SequencerTransactionStatus,
    pub failure_reason: Option<String>,
}
