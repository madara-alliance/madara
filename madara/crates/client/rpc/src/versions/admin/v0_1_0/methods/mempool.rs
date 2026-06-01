use mp_rpc::admin::MempoolNonceFilter;
use mp_transactions::validated::ValidatedTransaction;

pub(super) fn matches_nonce_filter(transaction: &ValidatedTransaction, nonce_filter: MempoolNonceFilter) -> bool {
    let nonce = transaction.transaction.nonce();
    nonce_filter.nonce_after.is_none_or(|lower| nonce > lower)
        && nonce_filter.nonce_before.is_none_or(|upper| nonce < upper)
}
