//! A [proptest], or property test, is a hypothesis-based test where a function
//! is repeatedly tested against procedurally generated inputs. This continues
//! until either:
//!
//! - Enough success cases have passed.
//! - A failure case has been found.
//!
//! If a failure case is detected, [proptest] will attempt to regress that case
//! into a minimal reproducible example by simplifying ([shrinking]) the inputs.
//! Failure cases, alongside the seed used to generate these cases, are stored
//! inside `proptest-regressions`. These failure cases will _always be run_ at
//! the start of a new proptest to make sure the code functions against
//! historical bugs. DO NOT MANUALLY EDIT OR DELETE THIS FILE.
//!
//! We use [state machine testing] to validate the execution of the inner
//! mempool against a procedurally run state machine. Shrinking still applies,
//! so if a failure case is found, [proptest_state_machine] will automatically
//! shrink it down to a minimal state transition.
//!
//! > The code in this section is based off the [state machine heap] example
//! > in the proptest state machine repository.
//!
//! [shrinking]: https://proptest-rs.github.io/proptest/proptest/tutorial/shrinking-basics.html
//! [state machine testing]: https://proptest-rs.github.io/proptest/proptest/state-machine.html
//! [state machine heap]: https://github.com/proptest-rs/proptest/blob/main/proptest-state-machine/examples/state_machine_heap.rs

#![cfg(test)]

use super::*;
use ::proptest::prelude::*;
use mp_transactions::validated::TxTimestamp;
use proptest_derive::Arbitrary;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest};
use starknet_types_core::felt::Felt;
use std::time::Duration;

proptest_state_machine::prop_state_machine! {
    #![proptest_config(property_testing::ProptestConfig {
        // Enable verbose mode to make the state machine test print the
        // transitions for each case.
        verbose: 1,
        // The number of tests which need to be valid for this to pass.
        cases: 64,
        // Max duration (in milliseconds) for each generated case.
        timeout: 1_000,
        ..Default::default()
    })]

    /// Simulates transaction insertion and removal into [MempoolInner].
    ///
    /// Each iteration will simulate the insertion of between 1 and 256
    /// [MempoolTransaction]s into the mempool. Note that insertions happen
    /// twice as often as popping from the mempool.
    #[test]
    fn mempool_proptest(sequential 1..256 => MempoolInner);
}

pub struct MempoolStateMachine;

/// Transactions to insert into the [MempoolInner] during proptesting.
#[derive(Clone, Debug, Arbitrary)]
pub enum TxTy {
    Declare,
    DeployAccount,
    Invoke,
    L1Handler,
}

impl TxTy {
    fn tx(self, contract_address: Felt) -> blockifier::transaction::transaction_execution::Transaction {
        match self {
            Self::Declare => blockifier::transaction::transaction_execution::Transaction::Account(AccountTransaction {
                tx: starknet_api::executable_transaction::AccountTransaction::Declare(
                    starknet_api::executable_transaction::DeclareTransaction {
                        tx: starknet_api::transaction::DeclareTransaction::V0(
                            starknet_api::transaction::DeclareTransactionV0V1 {
                                sender_address: ContractAddress::try_from(contract_address).unwrap(),
                                ..Default::default()
                            },
                        ),
                        tx_hash: starknet_api::transaction::TransactionHash::default(),
                        class_info: starknet_api::contract_class::ClassInfo::new(
                            &starknet_api::contract_class::ContractClass::V0(
                                starknet_api::deprecated_contract_class::ContractClass::default(),
                            ),
                            0,
                            0,
                            starknet_api::contract_class::SierraVersion::DEPRECATED,
                        )
                        .unwrap(),
                    },
                ),
                execution_flags: blockifier::transaction::account_transaction::ExecutionFlags::default(),
            }),
            Self::DeployAccount => {
                blockifier::transaction::transaction_execution::Transaction::Account(AccountTransaction {
                    tx: starknet_api::executable_transaction::AccountTransaction::DeployAccount(
                        starknet_api::executable_transaction::DeployAccountTransaction {
                            tx: starknet_api::transaction::DeployAccountTransaction::V1(
                                starknet_api::transaction::DeployAccountTransactionV1::default(),
                            ),
                            tx_hash: starknet_api::transaction::TransactionHash::default(),
                            contract_address: ContractAddress::try_from(contract_address).unwrap(),
                        },
                    ),
                    execution_flags: blockifier::transaction::account_transaction::ExecutionFlags::default(),
                })
            }
            Self::Invoke => blockifier::transaction::transaction_execution::Transaction::Account(AccountTransaction {
                tx: starknet_api::executable_transaction::AccountTransaction::Invoke(
                    starknet_api::executable_transaction::InvokeTransaction {
                        tx: starknet_api::transaction::InvokeTransaction::V0(
                            starknet_api::transaction::InvokeTransactionV0 {
                                contract_address: ContractAddress::try_from(contract_address).unwrap(),
                                ..Default::default()
                            },
                        ),
                        tx_hash: starknet_api::transaction::TransactionHash::default(),
                    },
                ),
                execution_flags: blockifier::transaction::account_transaction::ExecutionFlags::default(),
            }),
            Self::L1Handler => blockifier::transaction::transaction_execution::Transaction::L1Handler(
                starknet_api::executable_transaction::L1HandlerTransaction {
                    tx: starknet_api::transaction::L1HandlerTransaction {
                        contract_address: ContractAddress::try_from(contract_address).unwrap(),
                        ..Default::default()
                    },
                    tx_hash: starknet_api::transaction::TransactionHash::default(),
                    paid_fee_on_l1: starknet_api::transaction::fields::Fee::default(),
                },
            ),
        }
    }
}

prop_compose! {
    /// This function defines a strategy: that is to say, a way for [proptest]
    /// to generate and _shrink_ values to a minimal reproducible case.
    ///
    /// You can learn more about [strategies] and [shrinking] in the
    /// [proptest book].
    ///
    /// [strategies]: https://proptest-rs.github.io/proptest/proptest/tutorial/macro-prop-compose.html
    /// [shrinking]: https://proptest-rs.github.io/proptest/proptest/tutorial/shrinking-basics.html
    /// [proptest book]: https://proptest-rs.github.io/proptest/proptest/index.html
    fn felt_upto(range: u128)(felt in 0..range) -> Felt {
        Felt::from(felt)
    }
}

prop_compose! {
    fn nonce_upto(range: u128)(felt in felt_upto(range)) -> Nonce {
        Nonce(felt)
    }
}

prop_compose! {
    fn mempool_transaction(contract_address: Felt)(
        txty in any::<TxTy>(),
        dt in -5400..5400i32,
        nonce in nonce_upto(4),
    ) -> MempoolTransaction {
        // IMPORTANT: we fiddle with the transaction arrival time so it
        // is anywhere between 1h30 before now, or 1h30 into the future. Note
        // that the transaction age limit for the mempool is set to 1h.
        let arrived_at = if dt < 0 {
            TxTimestamp::now().checked_sub(Duration::from_secs(dt.unsigned_abs() as u64)).unwrap()
        } else {
            TxTimestamp::now().checked_add(Duration::from_secs(dt as u64)).unwrap()
        };
        let tx = txty.tx(contract_address);

        // IMPORTANT: the nonce and the address of the contracts sending
        // them should be kept low or else we will never be popping
        // ready transactions.
        let nonce_next = nonce.try_increment().unwrap();

        MempoolTransaction { tx, arrived_at, converted_class: None, nonce, nonce_next }
    }
}

prop_compose! {
    /// This is a higher-order strategy, that is to say: a strategy which is
    /// generated from another strategy.
    ///
    /// Since the syntax used to generate this can be pretty confusing, I
    /// suggest you check out the section of the [proptest book] on [strategies]
    /// and [higher-order strategies].
    ///
    /// [proptest book]: https://proptest-rs.github.io/proptest/proptest/index.html
    /// [strategies]: https://proptest-rs.github.io/proptest/proptest/tutorial/macro-prop-compose.html
    /// [higher-order strategies]: https://proptest-rs.github.io/proptest/proptest/tutorial/higher-order.html
    fn mempool_transition_push(mempool: MempoolInner)(contract_address in felt_upto(100))(
        mut tx in mempool_transaction(contract_address),
        force in any::<bool>(),
    ) -> MempoolTransition {
        let target = mempool.nonce_cache_inner.get(&tx.contract_address()).copied().unwrap_or(Nonce(Felt::ZERO));

        // IMPORTANT: we fiddle with the generated nonce to make sure it is
        // always greater than any previously popped transactions for that
        // account. Normally this would be checked by the outer mempool, so we
        // can be sure that transactions with an invalid nonce would not be
        // inserted into the inner mempool anyway.
        if tx.nonce < target {
            tx.nonce = Nonce(*target + *tx.nonce);
            tx.nonce_next = tx.nonce.try_increment().unwrap();
        }
        let (nonce, nonce_next) = (tx.nonce, tx.nonce_next);

        let nonce_info = if nonce == Nonce(Felt::ZERO) || nonce == target {
            NonceInfo::ready(nonce, nonce_next)
        } else {
            NonceInfo::pending(nonce, nonce_next)
        };

        MempoolTransition::Push { tx, force, nonce_info }
    }
}

/// A valid state transitions for the proptest.
///
/// We can only either [pop] or [push] transactions into [MempoolInner]
///
/// [pop]: MempoolTransition::Pop
/// [push]: MempoolTransition::Push
#[derive(Clone, Debug)]
pub enum MempoolTransition {
    /// Removes the next ready transaction from the mempool, if any.
    Pop,
    /// Tries to add a transaction into the mempool.
    Push { tx: MempoolTransaction, force: bool, nonce_info: NonceInfo },
}

impl ReferenceStateMachine for MempoolStateMachine {
    type State = MempoolInner;
    type Transition = MempoolTransition;

    fn init_state() -> BoxedStrategy<Self::State> {
        Just(MempoolInner::new(MempoolLimits {
            // Transactions in the mempool cannot be older than 1h
            max_age: Some(Duration::from_secs(3_600)),
            ..MempoolLimits::for_testing()
        }))
        .boxed()
    }

    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        // Insertions happen twice as often as removals
        prop_oneof![
            1 => Just(MempoolTransition::Pop),
            2 => mempool_transition_push(state.clone()),
        ]
        .boxed()
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition.to_owned() {
            MempoolTransition::Pop => {
                state.pop_next();
            }
            MempoolTransition::Push { tx, force, nonce_info } => {
                // We do not check invariants here
                let update_limits = true;
                let _ = state.insert_tx(tx, force, update_limits, nonce_info);
            }
        }

        state
    }
}

impl StateMachineTest for MempoolInner {
    type SystemUnderTest = Self;
    type Reference = MempoolStateMachine;

    fn init_test(_ref_state: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        // Transactions cannot live longer than 1h
        Self::new(MempoolLimits { max_age: Some(Duration::from_secs(3_600)), ..MempoolLimits::for_testing() })
    }

    fn apply(
        mut state: Self::SystemUnderTest,
        _ref_state: &<Self::Reference as ReferenceStateMachine>::State,
        transition: <Self::Reference as ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            MempoolTransition::Pop => {
                state.pop_next();
            }
            MempoolTransition::Push { tx, force, nonce_info } => {
                // tx info
                let readiness = nonce_info.readiness.clone();
                let nonce = nonce_info.nonce;
                let contract_address = **tx.contract_address();
                let tx_hash = tx.tx_hash();

                // age check
                let arrived_at = tx.arrived_at;
                let now = TxTimestamp::now();
                let too_old = if arrived_at < now {
                    now.duration_since(arrived_at).unwrap() > state.limiter.config.max_age.unwrap_or(Duration::MAX)
                } else {
                    false
                };

                // apply the actual state change
                let update_limits = true;
                let res = state.insert_tx(tx, force, update_limits, nonce_info);

                // check for invalid state
                match res {
                    Ok(_) => match readiness {
                        NonceStatus::Ready => assert!(
                            state.nonce_is_ready(contract_address, nonce),
                            "tx at {contract_address:x?} and {nonce:?} should be ready"
                        ),
                        NonceStatus::Pending => assert!(
                            state.nonce_is_pending(contract_address, nonce),
                            "tx at {contract_address:x?} and {nonce:?} should be pending"
                        ),
                    },
                    Err(err) => {
                        assert!(!force, "Force-insertions should not error!");
                        match err {
                            TxInsertionError::NonceConflict => assert!(
                                state.nonce_exists(contract_address, nonce),
                                "tx at {contract_address:x?} and {nonce:?} should already exist"
                            ),
                            TxInsertionError::DuplicateTxn => {
                                assert!(state.tx_hash_exists(contract_address, nonce, tx_hash))
                            }
                            TxInsertionError::Limit(_) => {
                                assert!(
                                    too_old,
                                    "Incorrectly marked transaction at {contract_address:x?} and P{nonce:?} as too old"
                                )
                            }
                        }
                    }
                }
            }
        }
        state
    }

    fn check_invariants(_state: &Self::SystemUnderTest, ref_state: &<Self::Reference as ReferenceStateMachine>::State) {
        ref_state.check_invariants();
    }
}
