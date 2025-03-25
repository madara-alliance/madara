#[cfg(test)]
mod block_tests {
    use super::super::common::temp_db::temp_db;
    use super::super::common::*;
    use crate::db_block_id::{DbBlockIdResolvable, RawDbBlockId};
    use crate::{block_db::TxIndex, db_block_id::DbBlockId};
    use mp_block::{BlockId, Header};
    use mp_chain_config::ChainConfig;
    use mp_convert::felt;

    #[tokio::test]
    async fn test_chain_info() {
        let db = temp_db().await;
        let chain_config = db.backend().chain_config();
        assert_eq!(chain_config.chain_id, ChainConfig::madara_test().chain_id);
    }

    #[tokio::test]
    async fn test_block_id() {
        let db = temp_db().await;
        let backend = db.backend();

        let block = finalized_block_zero(Header::default());
        let block_hash = block.info.block_hash().unwrap();
        let state_diff = finalized_state_diff_zero();

        backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();
        backend.store_block(pending_block_one(), pending_state_diff_one(), vec![]).unwrap();

        assert_eq!(backend.resolve_block_id(&BlockId::Hash(block_hash)).unwrap().unwrap(), RawDbBlockId::Number(0));
        assert_eq!(backend.resolve_block_id(&BlockId::Number(0)).unwrap().unwrap(), RawDbBlockId::Number(0));
        assert_eq!(backend.resolve_block_id(&DbBlockId::Pending).unwrap().unwrap(), RawDbBlockId::Pending);
    }

    #[tokio::test]
    async fn test_block_id_not_found() {
        let db = temp_db().await;
        let backend = db.backend();

        assert!(backend.resolve_block_id(&BlockId::Hash(felt!("0x0"))).unwrap().is_none());
    }

    #[tokio::test]
    async fn test_store_block() {
        const BLOCK_ID_0: RawDbBlockId = RawDbBlockId::Number(0);

        let db = temp_db().await;
        let backend = db.backend();

        assert!(backend.get_block(&BLOCK_ID_0).unwrap().is_none());

        let block = finalized_block_zero(Header::default());
        let state_diff = finalized_state_diff_zero();

        backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

        assert_eq!(backend.get_block_hash(&BLOCK_ID_0).unwrap().unwrap(), block.info.block_hash().unwrap());
        assert_eq!(BLOCK_ID_0.resolve_db_block_id(backend).unwrap().unwrap(), BLOCK_ID_0);
        assert_eq!(backend.get_block_info(&BLOCK_ID_0).unwrap().unwrap(), block.info);
        assert_eq!(backend.get_block_inner(&BLOCK_ID_0).unwrap().unwrap(), block.inner);
        assert_eq!(backend.get_block(&BLOCK_ID_0).unwrap().unwrap(), block);
        assert_eq!(backend.get_block_n(&BLOCK_ID_0).unwrap().unwrap(), 0);
        assert_eq!(backend.get_block_state_diff(&BLOCK_ID_0).unwrap().unwrap(), state_diff);
    }

    #[tokio::test]
    async fn test_store_pending_block() {
        const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

        let db = temp_db().await;
        let backend = db.backend();

        assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().is_some()); // Pending block should always be there

        let block = pending_block_one();
        let state_diff = pending_state_diff_one();

        backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

        assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
        assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block.info);
        assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block.inner);
        assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block);
        assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
    }

    #[tokio::test]
    async fn test_erase_pending_block() {
        const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

        let db = temp_db().await;
        let backend = db.backend();

        backend.store_block(finalized_block_zero(Header::default()), finalized_state_diff_zero(), vec![]).unwrap();
        backend.store_block(pending_block_one(), pending_state_diff_one(), vec![]).unwrap();
        backend.clear_pending_block().unwrap();

        assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap().inner.transactions.is_empty());
        assert_eq!(
            backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap().info.as_pending().unwrap().header.parent_block_hash,
            finalized_block_zero(Header::default()).info.as_closed().unwrap().block_hash,
            "fake pending block parent hash must match with latest block in db"
        );

        backend.store_block(finalized_block_one(), finalized_state_diff_one(), vec![]).unwrap();

        let block_pending = pending_block_two();
        let state_diff = pending_state_diff_two();
        backend.store_block(block_pending.clone(), state_diff.clone(), vec![]).unwrap();

        assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
        assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block_pending.info);
        assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block_pending.inner);
        assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block_pending);
        assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
    }

    #[tokio::test]
    async fn test_store_latest_block() {
        let db = temp_db().await;
        let backend = db.backend();

        backend.store_block(finalized_block_zero(Header::default()), finalized_state_diff_zero(), vec![]).unwrap();

        let latest_block = finalized_block_one();
        backend.store_block(latest_block.clone(), finalized_state_diff_one(), vec![]).unwrap();

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_latest_confirmed_block() {
        let db = temp_db().await;
        let backend = db.backend();

        assert!(backend.get_l1_last_confirmed_block().unwrap().is_none());

        backend.write_last_confirmed_block(0).unwrap();

        assert_eq!(backend.get_l1_last_confirmed_block().unwrap().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_store_block_transactions() {
        let db = temp_db().await;
        let backend = db.backend();

        let block = finalized_block_zero(Header::default());
        let state_diff = finalized_state_diff_zero();

        backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

        let tx_hash_1 = block.info.tx_hashes()[1];
        assert_eq!(backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(), (block.info.clone(), TxIndex(1)));
        assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block, TxIndex(1)));
    }

    #[tokio::test]
    async fn test_store_block_transactions_pending() {
        let db = temp_db().await;
        let backend = db.backend();

        backend.store_block(finalized_block_zero(Header::default()), finalized_state_diff_zero(), vec![]).unwrap();

        let block_pending = pending_block_one();
        backend.store_block(block_pending.clone(), pending_state_diff_one(), vec![]).unwrap();

        let tx_hash_1 = block_pending.info.tx_hashes()[1];
        assert_eq!(
            backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(),
            (block_pending.info.clone(), TxIndex(1))
        );
        assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block_pending, TxIndex(1)));
    }
}
