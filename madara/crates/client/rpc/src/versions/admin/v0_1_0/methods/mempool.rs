use mp_rpc::admin::MempoolNonceFilter;
use mp_transactions::validated::ValidatedTransaction;

#[cfg(feature = "mempool-intake-admin")]
use crate::{versions::admin::v0_1_0::MadaraMempoolRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
#[cfg(feature = "mempool-intake-admin")]
use jsonrpsee::core::{async_trait, RpcResult};

pub(super) fn matches_nonce_filter(transaction: &ValidatedTransaction, nonce_filter: MempoolNonceFilter) -> bool {
    if nonce_filter != MempoolNonceFilter::default() && transaction.sender_contract_address().is_none() {
        return false;
    }

    let nonce = transaction.transaction.nonce();
    nonce_filter.nonce_after.is_none_or(|lower| nonce > lower)
        && nonce_filter.nonce_before.is_none_or(|upper| nonce < upper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_convert::Felt;
    use mp_transactions::{
        validated::TxTimestamp, InvokeTransaction, InvokeTransactionV1, L1HandlerTransaction, Transaction,
    };

    fn validated(transaction: Transaction, contract_address: Felt) -> ValidatedTransaction {
        ValidatedTransaction {
            transaction,
            paid_fee_on_l1: None,
            contract_address,
            arrived_at: TxTimestamp(0),
            declared_class: None,
            hash: Felt::ZERO,
            charge_fee: true,
        }
    }

    #[test]
    fn nonce_filter_matches_only_account_transaction_nonces() {
        let account_tx = validated(
            Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                sender_address: Felt::from(1_u64),
                calldata: Default::default(),
                max_fee: Felt::ZERO,
                signature: Default::default(),
                nonce: Felt::from(3_u64),
            })),
            Felt::from(1_u64),
        );
        let l1_handler_tx = validated(
            Transaction::L1Handler(L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 3,
                contract_address: Felt::from(2_u64),
                entry_point_selector: Felt::ZERO,
                calldata: Default::default(),
            }),
            Felt::from(2_u64),
        );
        let nonce_filter =
            MempoolNonceFilter { nonce_after: Some(Felt::from(2_u64)), nonce_before: Some(Felt::from(4_u64)) };

        assert!(matches_nonce_filter(&account_tx, nonce_filter));
        assert!(!matches_nonce_filter(&l1_handler_tx, nonce_filter));
        assert!(matches_nonce_filter(&l1_handler_tx, MempoolNonceFilter::default()));
    }
}

#[cfg(feature = "mempool-intake-admin")]
#[async_trait]
impl MadaraMempoolRpcApiV0_1_0Server for Starknet {
    async fn set_mempool_intake(&self, enabled: bool) -> RpcResult<()> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .set_mempool_intake(enabled)
            .map_err(StarknetRpcApiError::from)?)
    }
}
