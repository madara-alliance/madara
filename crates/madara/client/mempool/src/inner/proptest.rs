#![cfg(test)]

use super::*;
use ::proptest::prelude::*;
use blockifier::{
    execution::contract_class::ClassInfo,
    test_utils::{contracts::FeatureContract, CairoVersion},
    transaction::{transaction_execution::Transaction, transaction_types::TransactionType},
};
use mc_exec::execution::TxInfo;
use mp_convert::ToFelt;
use proptest_derive::Arbitrary;
use starknet_api::{
    core::{calculate_contract_address, ChainId, Nonce},
    data_availability::DataAvailabilityMode,
    transaction::{
        ContractAddressSalt, DeclareTransactionV3, DeployAccountTransactionV3, InvokeTransactionV3, Resource,
        ResourceBounds, ResourceBoundsMapping, TransactionHash, TransactionHasher, TransactionVersion,
    },
};
use starknet_types_core::felt::Felt;

use blockifier::abi::abi_utils::selector_from_name;
use starknet_api::transaction::Fee;
use std::{
    collections::HashSet,
    fmt,
    time::{Duration, SystemTime},
};

lazy_static::lazy_static! {
    static ref DUMMY_CLASS: ClassInfo = {
        let dummy_contract_class = FeatureContract::TestContract(CairoVersion::Cairo1);
        ClassInfo::new(&dummy_contract_class.get_class(), 100, 100).unwrap()
    };

    static ref NOW: SystemTime = SystemTime::now();
}

struct Insert(MempoolTransaction, /* force */ bool);
impl fmt::Debug for Insert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Insert(ty={:?},arrived_at={:?},tx_hash={:?},contract_address={:?},nonce={:?},force={:?})",
            self.0.tx.tx_type(),
            self.0.arrived_at,
            self.0.tx_hash(),
            self.0.contract_address(),
            self.0.nonce(),
            self.1,
        )
    }
}
impl Arbitrary for Insert {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        #[derive(Debug, Arbitrary)]
        enum TxTy {
            Declare,
            DeployAccount,
            InvokeFunction,
            L1Handler,
        }

        <(TxTy, u8, u8, u8, bool)>::arbitrary()
            .prop_map(|(ty, arrived_at, contract_address, nonce, force)| {
                let arrived_at = *NOW + Duration::from_millis(arrived_at.into());
                let contract_addr = ContractAddress::try_from(Felt::from(contract_address)).unwrap();
                let nonce = Nonce(Felt::from(nonce));

                let resource_bounds = ResourceBoundsMapping(
                    [
                        (Resource::L1Gas, ResourceBounds { max_amount: 5, max_price_per_unit: 5 }),
                        (Resource::L2Gas, ResourceBounds { max_amount: 5, max_price_per_unit: 5 }),
                    ]
                    .into(),
                );

                let tx = match ty {
                    TxTy::Declare => starknet_api::transaction::Transaction::Declare(
                        starknet_api::transaction::DeclareTransaction::V3(DeclareTransactionV3 {
                            resource_bounds,
                            tip: Default::default(),
                            signature: Default::default(),
                            nonce,
                            class_hash: Default::default(),
                            compiled_class_hash: Default::default(),
                            sender_address: contract_addr,
                            nonce_data_availability_mode: DataAvailabilityMode::L1,
                            fee_data_availability_mode: DataAvailabilityMode::L1,
                            paymaster_data: Default::default(),
                            account_deployment_data: Default::default(),
                        }),
                    ),
                    TxTy::DeployAccount => starknet_api::transaction::Transaction::DeployAccount(
                        starknet_api::transaction::DeployAccountTransaction::V3(DeployAccountTransactionV3 {
                            resource_bounds,
                            tip: Default::default(),
                            signature: Default::default(),
                            nonce,
                            class_hash: Default::default(),
                            nonce_data_availability_mode: DataAvailabilityMode::L1,
                            fee_data_availability_mode: DataAvailabilityMode::L1,
                            paymaster_data: Default::default(),
                            contract_address_salt: ContractAddressSalt(contract_addr.to_felt()),
                            constructor_calldata: Default::default(),
                        }),
                    ),
                    TxTy::InvokeFunction => starknet_api::transaction::Transaction::Invoke(
                        starknet_api::transaction::InvokeTransaction::V3(InvokeTransactionV3 {
                            resource_bounds,
                            tip: Default::default(),
                            signature: Default::default(),
                            nonce,
                            sender_address: contract_addr,
                            calldata: Default::default(),
                            nonce_data_availability_mode: DataAvailabilityMode::L1,
                            fee_data_availability_mode: DataAvailabilityMode::L1,
                            paymaster_data: Default::default(),
                            account_deployment_data: Default::default(),
                        }),
                    ),
                    // TODO: maybe update the values?
                    TxTy::L1Handler => starknet_api::transaction::Transaction::L1Handler(
                        starknet_api::transaction::L1HandlerTransaction {
                            version: TransactionVersion::ZERO,
                            nonce,
                            contract_address: contract_addr,
                            entry_point_selector: selector_from_name("l1_handler_set_value"),
                            calldata: Default::default(),
                        },
                    ),
                };

                let deployed = if let starknet_api::transaction::Transaction::DeployAccount(tx) = &tx {
                    Some(
                        calculate_contract_address(
                            tx.contract_address_salt(),
                            Default::default(),
                            &Default::default(),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                } else {
                    None
                };

                // providing dummy l1 gas for now
                let l1_gas_paid = match &tx {
                    starknet_api::transaction::Transaction::L1Handler(_) => Some(Fee(1)),
                    _ => None,
                };

                let tx_hash = tx.calculate_transaction_hash(&ChainId::Mainnet, &TransactionVersion::THREE).unwrap();

                let tx = Transaction::from_api(tx, tx_hash, Some(DUMMY_CLASS.clone()), l1_gas_paid, deployed, false)
                    .unwrap();

                Insert(MempoolTransaction { tx, arrived_at, converted_class: None }, force)
            })
            .boxed()
    }
}

#[derive(Debug, Arbitrary)]
enum Operation {
    Insert(Insert),
    Pop,
}

#[derive(Debug, Arbitrary)]
struct MempoolInvariantsProblem(Vec<Operation>);
impl MempoolInvariantsProblem {
    fn check(&self) {
        tracing::debug!("\n\n\n\n\nCase: {:#?}", self);
        let mut mempool = MempoolInner::new(MempoolLimits::for_testing());
        mempool.check_invariants();

        let mut inserted = HashSet::new();
        let mut inserted_contract_nonce_pairs = HashSet::new();
        let mut new_contracts = HashSet::new();

        let handle_pop = |res: Option<MempoolTransaction>,
                          inserted: &mut HashSet<TransactionHash>,
                          inserted_contract_nonce_pairs: &mut HashSet<(Nonce, ContractAddress)>,
                          new_contracts: &mut HashSet<ContractAddress>| {
            if let Some(res) = &res {
                let removed = inserted.remove(&res.tx_hash());
                assert!(removed);
                let removed = inserted_contract_nonce_pairs.remove(&(res.nonce(), res.contract_address()));
                assert!(removed);

                if res.tx.tx_type() == TransactionType::DeployAccount {
                    let _removed = new_contracts.remove(&res.contract_address());
                    // there can be multiple deploy_account txs.
                    // assert!(removed)
                }
            } else {
                assert!(inserted.is_empty())
            }
            tracing::trace!("Popped {:?}", res.map(|el| Insert(el, false)));
        };

        for op in &self.0 {
            match op {
                Operation::Insert(insert) => {
                    let force = insert.1;
                    tracing::trace!("Insert {:?}", insert);
                    let res = mempool.insert_tx(insert.0.clone(), insert.1, true);

                    let expected = if !force
                        && inserted_contract_nonce_pairs.contains(&(insert.0.nonce(), insert.0.contract_address()))
                    {
                        if inserted.contains(&insert.0.tx_hash()) {
                            Err(TxInsersionError::DuplicateTxn)
                        } else {
                            Err(TxInsersionError::NonceConflict)
                        }
                    } else {
                        Ok(())
                    };

                    assert_eq!(expected, res);

                    if expected.is_ok() {
                        if insert.0.tx.tx_type() == TransactionType::DeployAccount {
                            new_contracts.insert(insert.0.contract_address());
                        }
                        inserted.insert(insert.0.tx_hash());
                        inserted_contract_nonce_pairs.insert((insert.0.nonce(), insert.0.contract_address()));
                    }

                    tracing::trace!("Result {:?}", res);
                }
                Operation::Pop => {
                    tracing::trace!("Pop");
                    let res = mempool.pop_next();
                    handle_pop(res, &mut inserted, &mut inserted_contract_nonce_pairs, &mut new_contracts);
                }
            }
            tracing::trace!("State: {mempool:#?}");
            mempool.check_invariants();
        }

        loop {
            tracing::trace!("Pop");
            let Some(res) = mempool.pop_next() else { break };
            handle_pop(Some(res), &mut inserted, &mut inserted_contract_nonce_pairs, &mut new_contracts);
            tracing::trace!("State: {mempool:#?}");
            mempool.check_invariants();
        }
        assert!(inserted.is_empty());
        assert!(inserted_contract_nonce_pairs.is_empty());
        assert!(new_contracts.is_empty());
        tracing::trace!("Done :)");
    }
}

::proptest::proptest! {
    #[tracing_test::traced_test]
    #[test]
    fn proptest_mempool(pb in any::<MempoolInvariantsProblem>()) {
        pb.check();
    }
}
