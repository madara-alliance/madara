#![cfg(test)]

use crate::{limits::MempoolLimitReached, tx::ScoreFunction, InnerMempool, InnerMempoolConfig, TxInsertionError};
use assert_matches::assert_matches;
use mp_convert::{Felt, ToFelt};
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use proptest::strategy::Strategy;
use rstest::{fixture, rstest};
use starknet_api::{core::Nonce, felt, transaction::TransactionHash};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

mod mempool_proptest;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TestTx {
    pub nonce: Felt,
    pub contract_address: Felt,
    pub arrived_at: u64,
    pub tip: Option<u64>,
    pub tx_hash: Felt,
    pub is_declare: bool,
}

impl From<TestTx> for ValidatedTransaction {
    fn from(value: TestTx) -> Self {
        Self {
            transaction: if value.is_declare {
                mp_transactions::Transaction::Declare(if let Some(tip) = value.tip {
                    mp_transactions::DeclareTransaction::V3(mp_transactions::DeclareTransactionV3 {
                        sender_address: value.contract_address,
                        compiled_class_hash: Felt::ZERO,
                        signature: Default::default(),
                        nonce: value.nonce,
                        class_hash: Default::default(),
                        resource_bounds: Default::default(),
                        tip,
                        paymaster_data: Default::default(),
                        account_deployment_data: Default::default(),
                        nonce_data_availability_mode: Default::default(),
                        fee_data_availability_mode: Default::default(),
                    })
                } else {
                    mp_transactions::DeclareTransaction::V1(mp_transactions::DeclareTransactionV1 {
                        sender_address: value.contract_address,
                        nonce: value.nonce,
                        class_hash: Default::default(),
                        max_fee: Default::default(),
                        signature: Default::default(),
                    })
                })
            } else {
                mp_transactions::Transaction::Invoke(if let Some(tip) = value.tip {
                    mp_transactions::InvokeTransaction::V3(mp_transactions::InvokeTransactionV3 {
                        sender_address: value.contract_address,
                        calldata: Default::default(),
                        signature: Default::default(),
                        nonce: value.nonce,
                        resource_bounds: Default::default(),
                        tip,
                        paymaster_data: Default::default(),
                        account_deployment_data: Default::default(),
                        nonce_data_availability_mode: Default::default(),
                        fee_data_availability_mode: Default::default(),
                    })
                } else {
                    mp_transactions::InvokeTransaction::V1(mp_transactions::InvokeTransactionV1 {
                        sender_address: value.contract_address,
                        nonce: value.nonce,
                        max_fee: Default::default(),
                        signature: Default::default(),
                        calldata: Default::default(),
                    })
                })
            },
            paid_fee_on_l1: None,
            contract_address: value.contract_address,
            arrived_at: TxTimestamp(value.arrived_at),
            declared_class: None,
            hash: value.tx_hash,
        }
    }
}

impl From<ValidatedTransaction> for TestTx {
    fn from(value: ValidatedTransaction) -> Self {
        Self {
            nonce: value.transaction.nonce(),
            contract_address: value.contract_address,
            arrived_at: value.arrived_at.0,
            tip: value.transaction.tip(),
            tx_hash: value.hash,
            is_declare: value.transaction.as_declare().is_some(),
        }
    }
}

pub struct MempoolTester {
    inner: InnerMempool,
    added_txs: HashSet<TestTx>,
    current_time: u64,
}

impl MempoolTester {
    pub fn new(config: InnerMempoolConfig) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        Self { inner: InnerMempool::new(config), added_txs: Default::default(), current_time: 0 }
    }

    fn check_invariants(&self) {
        tracing::debug!("-> STATE: {:#?}", self.inner);
        self.inner.check_invariants();
        let expected = self.inner.transactions().cloned().map(Into::into).collect::<HashSet<_>>();
        assert_eq!(expected, self.added_txs);
    }

    pub fn set_current_time(&mut self, current_time: u64) {
        self.current_time = current_time;
    }

    pub fn insert_tx(&mut self, tx: TestTx, account_nonce: Felt) -> Result<(), TxInsertionError> {
        let mut removed = vec![];
        tracing::debug!("TRY INSERT {tx:?} < {account_nonce:?}");
        let res =
            self.inner.insert_tx(TxTimestamp(self.current_time), tx.clone().into(), Nonce(account_nonce), &mut removed);
        for el in removed {
            let removed = self.added_txs.remove(&el.into());
            assert!(removed);
        }
        if res.is_ok() {
            let inserted = self.added_txs.insert(tx);
            assert!(inserted);
        }
        self.check_invariants();
        tracing::debug!("RES INSERT {res:?}");
        res
    }

    pub fn update_account_nonce(&mut self, contract_address: Felt, account_nonce: Felt) {
        let mut removed = vec![];
        tracing::debug!("UPDATE_NONCE {contract_address:#x} => {account_nonce:#x}");
        self.inner.update_account_nonce(&contract_address.try_into().unwrap(), &Nonce(account_nonce), &mut removed);
        for el in removed {
            let removed = self.added_txs.remove(&el.into());
            assert!(removed);
        }
        self.check_invariants();
    }

    pub fn pop_next_ready(&mut self) -> Option<TestTx> {
        let ret = self.inner.pop_next_ready();
        tracing::debug!("POP_READY");
        if let Some(removed) = ret.clone() {
            let removed = self.added_txs.remove(&removed.into());
            assert!(removed);
        }
        let r = ret.map(Into::into);
        tracing::debug!("RES POP_READY => {r:?}");
        self.check_invariants();
        r
    }

    pub fn remove_all_ttl_exceeded_txs(&mut self) {
        let mut removed = vec![];
        tracing::debug!("REMOVE_TTL_TXS");
        self.inner.remove_all_ttl_exceeded_txs(TxTimestamp(self.current_time), &mut removed);
        for el in removed {
            let removed = self.added_txs.remove(&el.into());
            assert!(removed);
        }
        self.check_invariants();
    }

    pub fn get_transaction_by_hash(&self, tx_hash: Felt) -> Option<TestTx> {
        self.inner.get_transaction_by_hash(&TransactionHash(tx_hash)).cloned().map(Into::into)
    }
    pub fn contains_tx_by_hash(&self, tx_hash: Felt) -> bool {
        self.inner.contains_tx_by_hash(&TransactionHash(tx_hash))
    }

    pub fn transactions(&self) -> HashSet<TestTx> {
        self.inner.transactions().cloned().map(Into::into).collect()
    }
    /// Key: contract addr, value: nonce.
    pub fn account_nonces(&self) -> HashMap<Felt, Felt> {
        self.inner.account_nonces().map(|(k, v)| (k.to_felt(), v.to_felt())).collect()
    }
}

#[fixture]
pub fn fcfs_mempool(
    #[default(4)] max_transactions: usize,
    #[default(Some(2))] max_declare_transactions: Option<usize>,
    #[default(Duration::from_secs(20))] ttl: Duration,
) -> MempoolTester {
    MempoolTester::new(InnerMempoolConfig {
        score_function: ScoreFunction::Timestamp,
        max_transactions,
        max_declare_transactions,
        ttl: Some(ttl),
    })
}

#[fixture]
pub fn tip_mempool(
    #[default(4)] max_transactions: usize,
    #[default(0.1)] min_tip_bump: f64,
    #[default(Some(2))] max_declare_transactions: Option<usize>,
    #[default(Duration::from_secs(20))] ttl: Duration,
) -> MempoolTester {
    MempoolTester::new(InnerMempoolConfig {
        score_function: ScoreFunction::Tip { min_tip_bump },
        max_transactions,
        max_declare_transactions,
        ttl: Some(ttl),
    })
}

#[rstest]
fn test_valid_insertion(mut fcfs_mempool: MempoolTester) {
    let tx = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    assert_eq!(fcfs_mempool.transactions(), [].into());

    assert_matches!(fcfs_mempool.insert_tx(tx.clone(), felt!("0x1")), Ok(()));

    assert_eq!(fcfs_mempool.transactions(), [tx].into());
    assert_eq!(fcfs_mempool.account_nonces(), [(felt!("0x123"), felt!("0x1"))].into());
}

#[rstest]
fn test_insert_expired_ttl_fails(mut fcfs_mempool: MempoolTester) {
    fcfs_mempool.set_current_time(1000 + 20_000 + 1000); // Past TTL
    let expired_tx = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    assert_matches!(fcfs_mempool.insert_tx(expired_tx, felt!("0x1")), Err(TxInsertionError::TooOld { .. }));
    assert_eq!(fcfs_mempool.transactions(), [].into());
}

#[rstest]
fn test_insert_with_nonce_too_low_fails(mut fcfs_mempool: MempoolTester) {
    let tx = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };

    assert_matches!(
        fcfs_mempool.insert_tx(tx, felt!("0x2")), // Account nonce is 0x2, tx nonce is 0x1
        Err(TxInsertionError::NonceTooLow { account_nonce }) if account_nonce == Nonce(felt!("0x2"))
    );

    assert_eq!(fcfs_mempool.transactions(), [].into());
}

#[rstest]
fn test_insert_with_nonce_too_low_fails_account_exists(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"), // Lower than existing account nonce
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xdef"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x2")), Ok(()));

    assert_matches!(
        fcfs_mempool.insert_tx(tx2, felt!("0x2")),
        Err(TxInsertionError::NonceTooLow { account_nonce }) if account_nonce.to_felt() == felt!("0x2")
    );

    assert_eq!(fcfs_mempool.transactions(), [tx1].into());
}

#[rstest]
fn test_fcfs_replace_with_bigger_arrived_at_fails(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 2000, // Later arrival
        tip: None,
        tx_hash: felt!("0xdef"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2, felt!("0x1")), Err(TxInsertionError::NonceConflict));

    assert_eq!(fcfs_mempool.transactions(), [tx1].into());
}

#[rstest]
fn test_fcfs_replace_with_lower_arrived_at_works(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 2000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000, // Earlier arrival
        tip: None,
        tx_hash: felt!("0xdef"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1, felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));

    assert_eq!(fcfs_mempool.transactions(), [tx2].into());
}

#[rstest]
fn test_tip_mode_replace_with_lower_tip_fails(mut tip_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: Some(100),
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 2000,
        tip: Some(50), // Lower tip
        tx_hash: felt!("0xdef"),
        is_declare: false,
    };

    assert_matches!(tip_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));

    assert_matches!(tip_mempool.insert_tx(tx2, felt!("0x1")), Err(TxInsertionError::MinTipBump { .. }));

    assert_eq!(tip_mempool.transactions(), [tx1].into());
}

#[rstest]
fn test_tip_mode_replace_with_sufficient_tip_bump_works(mut tip_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: Some(100),
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 2000,
        tip: Some(115), // Tip bump of 15 = 15% > min_tip_bump of 10%
        tx_hash: felt!("0xdef"),
        is_declare: false,
    };

    assert_matches!(tip_mempool.insert_tx(tx1, felt!("0x1")), Ok(()));
    assert_matches!(tip_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));

    assert_eq!(tip_mempool.transactions(), [tx2].into());
}

#[rstest]
fn test_tip_mode_replace_with_insufficient_tip_bump_fails(mut tip_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: Some(100),
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 2000,
        tip: Some(109), // Tip bump of 9 < min_tip_bump of 10%
        tx_hash: felt!("0xdef"),
        is_declare: false,
    };

    assert_matches!(tip_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));

    assert_matches!(tip_mempool.insert_tx(tx2, felt!("0x1")), Err(TxInsertionError::MinTipBump { min_tip_bump: 0.1 }));

    assert_eq!(tip_mempool.transactions(), [tx1].into());
}

#[rstest]
fn test_replace_non_declare_with_declare_exceeding_limit_fails(mut fcfs_mempool: MempoolTester) {
    // Fill up declare limit (max_declare_transactions = 2)
    let declare1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: true,
    };
    let declare2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x200"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: true,
    };
    let invoke = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let declare_replacement = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 500, // Better timestamp
        tip: None,
        tx_hash: felt!("0xdef"),
        is_declare: true,
    };

    assert_matches!(fcfs_mempool.insert_tx(declare1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(declare2.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(invoke.clone(), felt!("0x1")), Ok(()));

    assert_matches!(
        fcfs_mempool.insert_tx(declare_replacement, felt!("0x1")),
        Err(TxInsertionError::Limit(MempoolLimitReached::MaxDeclareTransactions { max: 2 }))
    );

    assert_eq!(fcfs_mempool.transactions(), [declare1, declare2, invoke].into());
}

#[rstest]
fn test_insert_declare_with_high_nonce_fails(mut fcfs_mempool: MempoolTester) {
    let declare_tx = TestTx {
        nonce: felt!("0x2"), // Higher than account nonce
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: true,
    };

    assert_matches!(
        fcfs_mempool.insert_tx(declare_tx, felt!("0x1")), // Account nonce is 0x1
        Err(TxInsertionError::PendingDeclare)
    );

    assert_eq!(fcfs_mempool.transactions(), [].into());
}

#[rstest]
fn test_insert_duplicate_tx_hash_fails(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 500,
        tip: None,
        tx_hash: felt!("0xabc"), // Same hash
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));

    assert_matches!(fcfs_mempool.insert_tx(tx2, felt!("0x1")), Err(TxInsertionError::DuplicateTxn));

    assert_eq!(fcfs_mempool.transactions(), [tx1].into());
}

#[rstest]
fn test_insert_declare_when_limit_reached_fails(mut fcfs_mempool: MempoolTester) {
    // Fill up declare limit (max_declare_transactions = 2)
    let declare1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: true,
    };
    let declare2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x200"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: true,
    };
    let declare3 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x300"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: true,
    };

    assert_matches!(fcfs_mempool.insert_tx(declare1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(declare2.clone(), felt!("0x1")), Ok(()));

    assert_matches!(
        fcfs_mempool.insert_tx(declare3, felt!("0x1")),
        Err(TxInsertionError::Limit(MempoolLimitReached::MaxDeclareTransactions { .. }))
    );

    assert_eq!(fcfs_mempool.transactions(), [declare1, declare2].into());
}

#[rstest]
fn test_eviction_by_highest_nonce_distance(#[with(3)] mut fcfs_mempool: MempoolTester) {
    // Fill mempool to capacity (3 transactions)
    let tx1 = TestTx {
        // nonce distance: 3-1=2
        nonce: felt!("0x3"),
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        // nonce distance: 2-1=1
        nonce: felt!("0x2"),
        contract_address: felt!("0x200"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        // nonce distance: 4-1=3 (highest)
        nonce: felt!("0x4"),
        contract_address: felt!("0x300"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: false,
    };
    let new_tx = TestTx {
        // nonce distance: 2-1=1
        nonce: felt!("0x2"),
        contract_address: felt!("0x400"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x400"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx3, felt!("0x1")), Ok(()));

    // Should evict tx3 (highest nonce distance)
    assert_matches!(fcfs_mempool.insert_tx(new_tx.clone(), felt!("0x1")), Ok(()));

    assert_eq!(fcfs_mempool.transactions(), [tx1, tx2, new_tx].into());
}

#[rstest]
fn test_eviction_by_score_when_nonce_distance_equal(#[with(3)] mut fcfs_mempool: MempoolTester) {
    // All transactions have same nonce distance (2-1=1)
    let tx1 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x100"),
        arrived_at: 1000, // Earlier (better score in FCFS)
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x200"),
        arrived_at: 2000, // Later (worse score in FCFS)
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x300"),
        arrived_at: 1500,
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: false,
    };
    let new_tx = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x400"),
        arrived_at: 800, // Better score than all
        tip: None,
        tx_hash: felt!("0x400"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2, felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx3.clone(), felt!("0x1")), Ok(()));

    // Should evict tx2 (worst score - highest arrived_at)
    assert_matches!(fcfs_mempool.insert_tx(new_tx.clone(), felt!("0x1")), Ok(()));

    assert_eq!(fcfs_mempool.transactions(), [tx1, tx3, new_tx].into());
}

#[rstest]
fn test_eviction_fails_when_new_tx_has_highest_nonce_distance(#[with(3)] mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        // nonce distance: 2-1=1
        nonce: felt!("0x2"),
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        // nonce distance: 3-1=2
        nonce: felt!("0x3"),
        contract_address: felt!("0x200"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        // nonce distance: 4-1=3
        nonce: felt!("0x4"),
        contract_address: felt!("0x300"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: false,
    };
    let new_tx = TestTx {
        // nonce distance: 5-1=4 (highest)
        nonce: felt!("0x5"),
        contract_address: felt!("0x400"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x400"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx3.clone(), felt!("0x1")), Ok(()));

    assert_matches!(
        fcfs_mempool.insert_tx(new_tx, felt!("0x1")),
        Err(TxInsertionError::Limit(MempoolLimitReached::MaxTransactions { max: 3 }))
    );

    assert_eq!(fcfs_mempool.transactions(), [tx1, tx2, tx3].into());
}

#[rstest]
fn test_eviction_fails_when_new_tx_has_worse_score(#[with(3)] mut fcfs_mempool: MempoolTester) {
    // All have same nonce distance
    let tx1 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x200"),
        arrived_at: 1500,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x300"),
        arrived_at: 2000, // Worst existing score
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: false,
    };
    let new_tx = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x400"),
        arrived_at: 3000, // Even worse score
        tip: None,
        tx_hash: felt!("0x400"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx3.clone(), felt!("0x1")), Ok(()));

    assert_matches!(
        fcfs_mempool.insert_tx(new_tx, felt!("0x1")),
        Err(TxInsertionError::Limit(MempoolLimitReached::MaxTransactions { max: 3 }))
    );

    assert_eq!(fcfs_mempool.transactions(), [tx1, tx2, tx3].into());
}

#[rstest]
fn test_fcfs_pop_next_ready_order(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x100"),
        arrived_at: 2000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x200"),
        arrived_at: 1000, // Earlier (should pop first)
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x300"),
        arrived_at: 1500,
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1, felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx3.clone(), felt!("0x1")), Ok(()));

    assert_eq!(fcfs_mempool.pop_next_ready(), Some(tx2)); // Earliest first
    assert_eq!(fcfs_mempool.pop_next_ready(), Some(tx3)); // Then middle
}

#[rstest]
fn test_tip_mode_pop_next_ready_order(mut tip_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: Some(50),
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x200"),
        arrived_at: 2000,
        tip: Some(100), // Highest tip (should pop first)
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x300"),
        arrived_at: 1500,
        tip: Some(75),
        tx_hash: felt!("0x300"),
        is_declare: false,
    };

    assert_matches!(tip_mempool.insert_tx(tx1, felt!("0x1")), Ok(()));
    assert_matches!(tip_mempool.insert_tx(tx2.clone(), felt!("0x1")), Ok(()));
    assert_matches!(tip_mempool.insert_tx(tx3.clone(), felt!("0x1")), Ok(()));

    assert_eq!(tip_mempool.pop_next_ready(), Some(tx2)); // Highest tip first
    assert_eq!(tip_mempool.pop_next_ready(), Some(tx3)); // Then middle tip
}

#[rstest]
fn test_pop_next_ready_skips_not_ready_txs(mut fcfs_mempool: MempoolTester) {
    let ready_tx = TestTx {
        nonce: felt!("0x1"), // Matches account nonce
        contract_address: felt!("0x100"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let not_ready_tx = TestTx {
        nonce: felt!("0x2"), // Higher than account nonce
        contract_address: felt!("0x200"),
        arrived_at: 500, // Earlier but not ready
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(ready_tx.clone(), felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(not_ready_tx.clone(), felt!("0x1")), Ok(()));

    assert_eq!(fcfs_mempool.pop_next_ready(), Some(ready_tx));
    assert_eq!(fcfs_mempool.pop_next_ready(), None); // not_ready_tx still in mempool but not poppable
    assert_eq!(fcfs_mempool.transactions(), [not_ready_tx].into());
}

#[rstest]
fn test_update_account_nonce_removes_old_txs(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x2"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };
    let tx3 = TestTx {
        nonce: felt!("0x3"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x300"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1, felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2, felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx3.clone(), felt!("0x1")), Ok(()));

    // Update account nonce to 0x3 - should remove tx1 and tx2
    fcfs_mempool.update_account_nonce(felt!("0x123"), felt!("0x3"));

    assert_eq!(fcfs_mempool.transactions(), [tx3].into());
    assert_eq!(fcfs_mempool.account_nonces(), [(felt!("0x123"), felt!("0x3"))].into())
}

#[rstest]
fn test_update_account_nonce_not_found(mut fcfs_mempool: MempoolTester) {
    // Account is not in mempool, do nothing.
    fcfs_mempool.update_account_nonce(felt!("0x123"), felt!("0x3"));
    assert_eq!(fcfs_mempool.transactions(), [].into());
    assert_eq!(fcfs_mempool.account_nonces(), [].into())
}

#[rstest]
fn test_update_account_nonce_makes_tx_ready(mut fcfs_mempool: MempoolTester) {
    let tx = TestTx {
        nonce: felt!("0x2"), // Not ready initially
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx.clone(), felt!("0x1")), Ok(()));
    assert_eq!(fcfs_mempool.pop_next_ready(), None); // Not ready

    // Update account nonce to make tx ready
    fcfs_mempool.update_account_nonce(felt!("0x123"), felt!("0x2"));

    assert_eq!(fcfs_mempool.pop_next_ready(), Some(tx)); // Now ready
}

#[rstest]
fn test_update_account_nonce_makes_ready_tx_not_ready(mut fcfs_mempool: MempoolTester) {
    let tx1 = TestTx {
        nonce: felt!("0x2"), // Will be ready
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let tx2 = TestTx {
        nonce: felt!("0x3"), // Will not be ready
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(tx1.clone(), felt!("0x2")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(tx2.clone(), felt!("0x2")), Ok(()));
    assert_eq!(fcfs_mempool.pop_next_ready(), Some(tx1)); // tx1 was ready

    // Update account nonce backwards - tx2 becomes not ready
    fcfs_mempool.update_account_nonce(felt!("0x123"), felt!("0x1"));

    assert_eq!(fcfs_mempool.pop_next_ready(), None); // tx2 no longer ready
    assert_eq!(fcfs_mempool.transactions(), [tx2].into());
}

#[rstest]
fn test_remove_all_ttl_exceeded_txs(mut fcfs_mempool: MempoolTester) {
    let old_tx = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x100"),
        arrived_at: 1000, // Will be expired
        tip: None,
        tx_hash: felt!("0x100"),
        is_declare: false,
    };
    let recent_tx = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x200"),
        arrived_at: 25000, // Within TTL
        tip: None,
        tx_hash: felt!("0x200"),
        is_declare: false,
    };

    assert_matches!(fcfs_mempool.insert_tx(old_tx, felt!("0x1")), Ok(()));
    assert_matches!(fcfs_mempool.insert_tx(recent_tx.clone(), felt!("0x1")), Ok(()));

    // Set time to make old_tx exceed TTL (20 seconds = 20000ms)
    fcfs_mempool.set_current_time(30000);
    fcfs_mempool.remove_all_ttl_exceeded_txs();

    assert_eq!(fcfs_mempool.transactions(), [recent_tx].into());
}

#[rstest]
fn test_get_tx_by_hash(mut fcfs_mempool: MempoolTester) {
    let tx = TestTx {
        nonce: felt!("0x1"),
        contract_address: felt!("0x123"),
        arrived_at: 1000,
        tip: None,
        tx_hash: felt!("0xabc"),
        is_declare: false,
    };

    assert!(!fcfs_mempool.contains_tx_by_hash(felt!("0xabc")));
    assert_eq!(fcfs_mempool.get_transaction_by_hash(felt!("0xabc")), None);
    assert!(!fcfs_mempool.contains_tx_by_hash(felt!("0x999")));
    assert_eq!(fcfs_mempool.get_transaction_by_hash(felt!("0x999")), None);

    assert_matches!(fcfs_mempool.insert_tx(tx.clone(), felt!("0x1")), Ok(()));

    assert!(fcfs_mempool.contains_tx_by_hash(felt!("0xabc")));
    assert_eq!(fcfs_mempool.get_transaction_by_hash(felt!("0xabc")), Some(tx));
    assert!(!fcfs_mempool.contains_tx_by_hash(felt!("0x999")));
    assert_eq!(fcfs_mempool.get_transaction_by_hash(felt!("0x999")), None);
}
