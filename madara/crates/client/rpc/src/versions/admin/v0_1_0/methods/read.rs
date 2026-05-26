use super::matches_nonce_filter;
use crate::{versions::admin::v0_1_0::MadaraReadRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use blockifier::bouncer::BouncerWeights;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::{
    admin::{GetMempoolTxnHashesParams, GetMempoolTxnsParams, MempoolTxnHashInfo, MempoolTxnInfo},
    v0_10_2::TxnWithHashAndProofFacts,
};
use mp_transactions::TransactionWithHash;

fn ttl_to_ms(ttl: std::time::Duration) -> u64 {
    ttl.as_millis().try_into().unwrap_or(u64::MAX)
}

#[async_trait]
impl MadaraReadRpcApiV0_1_0Server for Starknet {
    async fn get_block_builtin_weights(&self, block_number: u64) -> RpcResult<BouncerWeights> {
        let block_view =
            self.backend.block_view_on_confirmed(block_number).ok_or(StarknetRpcApiError::BlockNotFound)?;
        let bouncer_weights = block_view.get_bouncer_weights().map_err(StarknetRpcApiError::from)?;
        Ok(bouncer_weights)
    }

    async fn get_mempool_txn_hashes(
        &self,
        params: Option<GetMempoolTxnHashesParams>,
    ) -> RpcResult<Vec<MempoolTxnHashInfo>> {
        let params = params.unwrap_or_default();
        let mempool = self.mempool.as_ref().ok_or(StarknetRpcApiError::UnimplementedMethod)?;
        let transactions = mempool
            .snapshot_transaction_hashes_matching(params.limit, params.include_ttl, |tx| {
                matches_nonce_filter(tx, params.nonce_filter)
            })
            .await;

        Ok(transactions
            .into_iter()
            .map(|snapshot| MempoolTxnHashInfo {
                transaction_hash: snapshot.transaction_hash,
                remaining_ttl_ms: snapshot.remaining_ttl.map(ttl_to_ms),
            })
            .collect())
    }

    async fn get_mempool_txns(&self, params: Option<GetMempoolTxnsParams>) -> RpcResult<Vec<MempoolTxnInfo>> {
        let params = params.unwrap_or_default();
        let mempool = self.mempool.as_ref().ok_or(StarknetRpcApiError::UnimplementedMethod)?;
        let transactions = mempool
            .snapshot_transactions_matching(params.limit, params.include_ttl, |tx| {
                matches_nonce_filter(tx, params.nonce_filter)
            })
            .await;

        Ok(transactions
            .into_iter()
            .map(|snapshot| {
                let validated_transaction = snapshot.transaction;
                let tx = TransactionWithHash::new(validated_transaction.transaction, validated_transaction.hash);
                MempoolTxnInfo {
                    transaction: TxnWithHashAndProofFacts {
                        transaction: tx.transaction.to_rpc_v0_10_2(false),
                        transaction_hash: tx.hash,
                    },
                    remaining_ttl_ms: if params.include_ttl { snapshot.remaining_ttl.map(ttl_to_ms) } else { None },
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_utils::TestTransactionProvider, versions::admin::v0_1_0::MadaraReadRpcApiV0_1_0Server, Starknet};
    use mc_db::MadaraBackend;
    use mc_mempool::{Mempool, MempoolConfig};
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_rpc::admin::MempoolNonceFilter;
    use mp_transactions::{
        validated::{TxTimestamp, ValidatedTransaction},
        InvokeTransaction, InvokeTransactionV1, Transaction,
    };
    use mp_utils::service::ServiceContext;
    use std::sync::Arc;
    use std::time::Duration;

    fn mempool_tx(sender: Felt, nonce: Felt, hash: Felt, arrived_at: u64) -> ValidatedTransaction {
        ValidatedTransaction {
            transaction: Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                sender_address: sender,
                calldata: vec![Felt::from(1_u64), Felt::from(2_u64)].into(),
                max_fee: Felt::from(3_u64),
                signature: vec![].into(),
                nonce,
            })),
            paid_fee_on_l1: None,
            contract_address: sender,
            arrived_at: TxTimestamp(arrived_at),
            declared_class: None,
            hash,
            charge_fee: true,
        }
    }

    fn make_starknet_with_chain_config(chain_config: ChainConfig) -> (Arc<Mempool>, Starknet) {
        let backend = MadaraBackend::open_for_testing(Arc::new(chain_config));
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
        let mut rpc = Starknet::new(
            backend,
            Arc::new(TestTransactionProvider),
            Default::default(),
            None,
            ServiceContext::new_for_testing(),
        );
        rpc.set_mempool(mempool.clone());
        (mempool, rpc)
    }

    fn make_starknet_with_mempool() -> (Arc<Mempool>, Starknet) {
        let mut chain_config = ChainConfig::madara_test();
        chain_config.mempool_ttl = Some(Duration::from_secs(60));
        make_starknet_with_chain_config(chain_config)
    }

    #[tokio::test]
    async fn get_mempool_txn_hashes_returns_oldest_first_and_optional_ttl() {
        assert_eq!(GetMempoolTxnHashesParams::default().limit, 100);

        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = mempool_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(101_u64), base);
        let tx2 = mempool_tx(Felt::from(22_u64), Felt::from(2_u64), Felt::from(202_u64), base + 1_000);

        mempool.accept_tx(tx2.clone()).await.unwrap();
        mempool.accept_tx(tx1.clone()).await.unwrap();

        let hashes = rpc.get_mempool_txn_hashes(None).await.unwrap();
        assert_eq!(hashes.iter().map(|entry| entry.transaction_hash).collect::<Vec<_>>(), vec![tx1.hash, tx2.hash]);
        assert!(hashes.iter().all(|entry| entry.remaining_ttl_ms.is_none()));

        let hashes_with_ttl = rpc
            .get_mempool_txn_hashes(Some(GetMempoolTxnHashesParams { include_ttl: true, ..Default::default() }))
            .await
            .unwrap();
        assert_eq!(
            hashes_with_ttl.iter().map(|entry| entry.transaction_hash).collect::<Vec<_>>(),
            vec![tx1.hash, tx2.hash]
        );
        assert!(hashes_with_ttl.iter().all(|entry| entry.remaining_ttl_ms.is_some()));

        let filtered_hashes = rpc
            .get_mempool_txn_hashes(Some(GetMempoolTxnHashesParams {
                nonce_filter: MempoolNonceFilter { nonce_after: Some(Felt::from(1_u64)), nonce_before: None },
                ..Default::default()
            }))
            .await
            .unwrap();
        assert_eq!(filtered_hashes.iter().map(|entry| entry.transaction_hash).collect::<Vec<_>>(), vec![tx2.hash]);
    }

    #[tokio::test]
    async fn get_mempool_txn_hashes_honors_explicit_limit() {
        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = mempool_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(101_u64), base);
        let tx2 = mempool_tx(Felt::from(22_u64), Felt::from(1_u64), Felt::from(202_u64), base + 1_000);
        let tx3 = mempool_tx(Felt::from(33_u64), Felt::from(2_u64), Felt::from(303_u64), base + 2_000);

        mempool.accept_tx(tx3.clone()).await.unwrap();
        mempool.accept_tx(tx1.clone()).await.unwrap();
        mempool.accept_tx(tx2.clone()).await.unwrap();

        let hashes = rpc
            .get_mempool_txn_hashes(Some(GetMempoolTxnHashesParams { limit: 2, ..Default::default() }))
            .await
            .unwrap();

        assert_eq!(hashes.iter().map(|entry| entry.transaction_hash).collect::<Vec<_>>(), vec![tx1.hash, tx2.hash]);
    }

    #[tokio::test]
    async fn get_mempool_txn_hashes_without_configured_ttl_reports_no_remaining_ttl() {
        let mut chain_config = ChainConfig::madara_test();
        chain_config.mempool_ttl = None;
        let (mempool, rpc) = make_starknet_with_chain_config(chain_config);
        let tx = mempool_tx(Felt::from(55_u64), Felt::ZERO, Felt::from(505_u64), TxTimestamp::now().0);

        mempool.accept_tx(tx).await.unwrap();

        let hashes = rpc
            .get_mempool_txn_hashes(Some(GetMempoolTxnHashesParams { include_ttl: true, ..Default::default() }))
            .await
            .unwrap();

        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0].remaining_ttl_ms, None);
    }

    #[tokio::test]
    async fn get_mempool_txns_returns_full_transactions() {
        assert_eq!(GetMempoolTxnsParams::default().limit, 100);

        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = mempool_tx(Felt::from(33_u64), Felt::ZERO, Felt::from(303_u64), base);
        let tx2 = mempool_tx(Felt::from(44_u64), Felt::from(3_u64), Felt::from(404_u64), base + 1_000);

        mempool.accept_tx(tx1.clone()).await.unwrap();
        mempool.accept_tx(tx2.clone()).await.unwrap();

        let transactions = rpc.get_mempool_txns(None).await.unwrap();
        assert_eq!(transactions.len(), 2);
        assert_eq!(transactions[0].transaction.transaction_hash, tx1.hash);
        assert_eq!(transactions[1].transaction.transaction_hash, tx2.hash);
        assert!(transactions.iter().all(|entry| entry.remaining_ttl_ms.is_none()));

        let transactions_with_ttl =
            rpc.get_mempool_txns(Some(GetMempoolTxnsParams { include_ttl: true, ..Default::default() })).await.unwrap();
        assert!(transactions_with_ttl.iter().all(|entry| entry.remaining_ttl_ms.is_some()));

        let filtered_transactions = rpc
            .get_mempool_txns(Some(GetMempoolTxnsParams {
                include_ttl: true,
                nonce_filter: MempoolNonceFilter {
                    nonce_after: Some(Felt::from(1_u64)),
                    nonce_before: Some(Felt::from(4_u64)),
                },
                ..Default::default()
            }))
            .await
            .unwrap();
        assert_eq!(filtered_transactions.len(), 1);
        assert_eq!(filtered_transactions[0].transaction.transaction_hash, tx2.hash);
        assert!(filtered_transactions[0].remaining_ttl_ms.is_some());
    }

    #[tokio::test]
    async fn get_mempool_txns_honors_explicit_limit() {
        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = mempool_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(101_u64), base);
        let tx2 = mempool_tx(Felt::from(22_u64), Felt::from(1_u64), Felt::from(202_u64), base + 1_000);
        let tx3 = mempool_tx(Felt::from(33_u64), Felt::from(2_u64), Felt::from(303_u64), base + 2_000);

        mempool.accept_tx(tx3.clone()).await.unwrap();
        mempool.accept_tx(tx1.clone()).await.unwrap();
        mempool.accept_tx(tx2.clone()).await.unwrap();

        let transactions =
            rpc.get_mempool_txns(Some(GetMempoolTxnsParams { limit: 2, ..Default::default() })).await.unwrap();

        assert_eq!(transactions.len(), 2);
        assert_eq!(transactions[0].transaction.transaction_hash, tx1.hash);
        assert_eq!(transactions[1].transaction.transaction_hash, tx2.hash);
    }
}
