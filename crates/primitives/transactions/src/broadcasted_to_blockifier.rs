use crate::{Transaction, TransactionWithHash};
use anyhow::Context;
use dp_class::{to_blockifier_class, ClassHash, ToCompiledClass};
use dp_convert::ToStarkFelt;
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;

pub fn broadcasted_to_blockifier(
    transaction: starknet_core::types::BroadcastedTransaction,
    chain_id: Felt,
) -> Result<blockifier::transaction::account_transaction::AccountTransaction, anyhow::Error> {
    let (class_info, class_hash) = match &transaction {
        starknet_core::types::BroadcastedTransaction::Declare(tx) => match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => (
                Some(blockifier::execution::contract_class::ClassInfo::new(
                    &to_blockifier_class(tx.contract_class.compile()?)?,
                    0,
                    0,
                )?),
                Some(tx.contract_class.class_hash().context("Failed to compute class hash on Legacy")?),
            ),
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => (
                Some(blockifier::execution::contract_class::ClassInfo::new(
                    &to_blockifier_class(tx.contract_class.compile()?)?,
                    tx.contract_class.sierra_program.len(),
                    tx.contract_class.abi.len(),
                )?),
                Some(tx.contract_class.class_hash()),
            ),
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => (
                Some(blockifier::execution::contract_class::ClassInfo::new(
                    &to_blockifier_class(tx.contract_class.compile()?)?,
                    tx.contract_class.sierra_program.len(),
                    tx.contract_class.abi.len(),
                )?),
                Some(tx.contract_class.class_hash()),
            ),
        },
        _ => (None, None),
    };

    let is_query = is_query(&transaction);
    let TransactionWithHash { transaction, hash } =
        TransactionWithHash::from_broadcasted(transaction, chain_id, class_hash);
    let deployed_address = match &transaction {
        Transaction::DeployAccount(tx) => Some(tx.calculate_contract_address()),
        _ => None,
    };
    let transaction: starknet_api::transaction::Transaction = (&transaction).try_into()?;

    if let blockifier::transaction::transaction_execution::Transaction::AccountTransaction(account_transaction) =
        blockifier::transaction::transaction_execution::Transaction::from_api(
            transaction,
            TransactionHash(hash.to_stark_felt()),
            class_info,
            None,
            deployed_address.map(|address| address.to_stark_felt().try_into().unwrap()),
            is_query,
        )?
    {
        Ok(account_transaction)
    } else {
        unreachable!()
    }
}

fn is_query(transaction: &starknet_core::types::BroadcastedTransaction) -> bool {
    match transaction {
        starknet_core::types::BroadcastedTransaction::Invoke(tx) => match tx {
            starknet_core::types::BroadcastedInvokeTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedInvokeTransaction::V3(tx) => tx.is_query,
        },
        starknet_core::types::BroadcastedTransaction::Declare(tx) => match tx {
            starknet_core::types::BroadcastedDeclareTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeclareTransaction::V2(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeclareTransaction::V3(tx) => tx.is_query,
        },
        starknet_core::types::BroadcastedTransaction::DeployAccount(tx) => match tx {
            starknet_core::types::BroadcastedDeployAccountTransaction::V1(tx) => tx.is_query,
            starknet_core::types::BroadcastedDeployAccountTransaction::V3(tx) => tx.is_query,
        },
    }
}
