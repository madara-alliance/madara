use mc_settlement_client::error::SettlementClientError;
use mp_rpc::v0_8_1::{L1TxnHash, TxnHashWithStatus};

use crate::{
    utils::display_internal_server_error,
    versions::user::v0_7_1::methods::read::get_transaction_status::get_transaction_status, Starknet,
    StarknetRpcApiError, StarknetRpcResult,
};

pub async fn get_messages_status(
    starknet: &Starknet,
    transaction_hash: L1TxnHash,
) -> StarknetRpcResult<Vec<TxnHashWithStatus>> {
    let transaction_hash = transaction_hash.into();
    let settlement_client = starknet.settlement_client().ok_or(StarknetRpcApiError::InternalServerError)?;
    let l1_handlers = settlement_client.get_messages_to_l2(transaction_hash).await.map_err(|e| match e {
        SettlementClientError::TxNotFound => StarknetRpcApiError::TxnHashNotFound,
        e => {
            display_internal_server_error(format!(
                "Error getting messages to L2 for L1 transaction {transaction_hash:?}: {e}"
            ));
            StarknetRpcApiError::InternalServerError
        }
    })?;

    let mut results = Vec::new();
    for msg in l1_handlers {
        let transaction_hash = msg.tx.compute_hash(starknet.chain_id(), false, false);
        let finality_status = get_transaction_status(starknet, transaction_hash).await?.finality_status;
        // TODO: Add failure_reason when we support it in get_transaction_status in v0.8.1.
        results.push(TxnHashWithStatus { transaction_hash, finality_status, failure_reason: None });
    }
    Ok(results)
}
