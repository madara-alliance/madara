use super::*;
use proptest::prelude::*;
use starknet_types_core::hash::{Pedersen, StarkHash};

fn arb_tx() -> impl Strategy<Value = TestTx> {
    (
        (0u64..=5).prop_map(|n| felt!(n)), // nonce: 0-5
        (0u64..=3).prop_map(|n| felt!(n)), // contract_address: 0-3
        (0u128..=5000),                    // arrived_at: 0-5000
        prop::option::of(10u64..=100),     // tip: Some(10-100) or None
        any::<bool>(),                     // is_declare
    )
        .prop_map(|(nonce, contract_address, arrived_at, tip, is_declare)| TestTx {
            nonce,
            contract_address,
            arrived_at,
            tip,
            tx_hash: Pedersen::hash_array(&[
                nonce,
                contract_address,
                arrived_at.into(),
                tip.map(Felt::from).unwrap_or_default(),
                (is_declare as u32).into(),
            ]),
            is_declare,
        })
}

#[derive(Debug, Clone)]
enum MempoolOp {
    InsertTx(TestTx, Felt),
    UpdateAccountNonce(Felt, Felt),
    PopNextReady,
    RemoveTtlExceeded,
    SetCurrentTime(u128),
}

fn arb_mempool_op() -> impl Strategy<Value = MempoolOp> {
    prop_oneof![
        (arb_tx(), (1u64..=5).prop_map(Felt::from)).prop_map(|(tx, nonce)| MempoolOp::InsertTx(tx, nonce)),
        ((1u64..=3).prop_map(Felt::from), (1u64..=5).prop_map(Felt::from))
            .prop_map(|(addr, nonce)| MempoolOp::UpdateAccountNonce(addr, nonce)),
        Just(MempoolOp::PopNextReady),
        Just(MempoolOp::RemoveTtlExceeded),
        (0u128..=10000).prop_map(MempoolOp::SetCurrentTime),
    ]
}

// Property: Internal data structures are always consistent (tip mode)
proptest! {
    #[test]
    fn prop_tip_invariants_always_hold(
        max_transactions in 1usize..=10,
        min_tip_bump in 0u128..=10,
        max_declare_transactions in prop::option::of(1usize..=5),
        ttl_millis in 0u64..=5000,
        ops in prop::collection::vec(arb_mempool_op(), 0..50)
    ) {
        let mut mempool = tip_mempool(
            max_transactions,
            min_tip_bump,
            max_declare_transactions,
            Duration::from_millis(ttl_millis)
        );
        tracing::info!("{ops:#?}");

        // Apply all operations
        for op in ops {
            match op {
                MempoolOp::InsertTx(tx, nonce) => {
                    let _ = mempool.insert_tx(tx, nonce);
                }
                MempoolOp::UpdateAccountNonce(addr, nonce) => {
                    mempool.update_account_nonce(addr, nonce);
                }
                MempoolOp::PopNextReady => {
                    mempool.pop_next_ready();
                }
                MempoolOp::RemoveTtlExceeded => {
                    mempool.remove_all_ttl_exceeded_txs();
                }
                MempoolOp::SetCurrentTime(time) => {
                    mempool.set_current_time(time);
                }
            }
        }

        // Invariants should always hold (this is checked automatically by MempoolTester)
        // If we get here without panicking, invariants held throughout
    }
}

// Property: Internal data structures are always consistent (FCFS mode)
proptest! {
    #[test]
    fn prop_fcfs_invariants_always_hold(
        max_transactions in 1usize..=10,
        max_declare_transactions in prop::option::of(1usize..=5),
        ttl_millis in 0u64..=5000,
        ops in prop::collection::vec(arb_mempool_op(), 0..50)
    ) {
        let mut mempool = fcfs_mempool(
            max_transactions,
            max_declare_transactions,
            Duration::from_millis(ttl_millis)
        );
        tracing::info!("{ops:#?}");

        // Apply all operations
        for op in ops {
            match op {
                MempoolOp::InsertTx(tx, nonce) => {
                    let _ = mempool.insert_tx(tx, nonce);
                }
                MempoolOp::UpdateAccountNonce(addr, nonce) => {
                    mempool.update_account_nonce(addr, nonce);
                }
                MempoolOp::PopNextReady => {
                    mempool.pop_next_ready();
                }
                MempoolOp::RemoveTtlExceeded => {
                    mempool.remove_all_ttl_exceeded_txs();
                }
                MempoolOp::SetCurrentTime(time) => {
                    mempool.set_current_time(time);
                }
            }
        }

        // Invariants should always hold (this is checked automatically by MempoolTester)
        // If we get here without panicking, invariants held throughout
    }
}
