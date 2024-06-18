use dp_convert::to_felt::ToFelt;
/// Here we transform starknet-api transactions into starknet-core trasnactions
use starknet_api::transaction::{
    DeclareTransaction, DeclareTransactionV0V1, DeclareTransactionV2, DeclareTransactionV3, DeployAccountTransaction,
    DeployAccountTransactionV1, DeployAccountTransactionV3, InvokeTransaction, InvokeTransactionV0,
    InvokeTransactionV1, InvokeTransactionV3, Resource, ResourceBoundsMapping, Transaction,
};
use starknet_core::types::{ResourceBounds, ResourceBoundsMapping as CoreResourceBoundsMapping};
use starknet_crypto::Felt;

pub fn to_starknet_core_tx(tx: &Transaction, transaction_hash: Felt) -> starknet_core::types::Transaction {
    match tx {
        Transaction::Declare(tx) => {
            let tx = match tx {
                DeclareTransaction::V0(DeclareTransactionV0V1 {
                    max_fee,
                    signature,
                    nonce: _,
                    class_hash,
                    sender_address,
                }) => starknet_core::types::DeclareTransaction::V0(starknet_core::types::DeclareTransactionV0 {
                    transaction_hash,
                    max_fee: Felt::from(max_fee.0),
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    class_hash: class_hash.0.to_felt(),
                    sender_address: sender_address.0.to_felt(),
                }),
                DeclareTransaction::V1(DeclareTransactionV0V1 {
                    max_fee,
                    signature,
                    nonce,
                    class_hash,
                    sender_address,
                    ..
                }) => starknet_core::types::DeclareTransaction::V1(starknet_core::types::DeclareTransactionV1 {
                    transaction_hash,
                    max_fee: Felt::from(max_fee.0),
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    nonce: nonce.to_felt(),
                    class_hash: class_hash.to_felt(),
                    sender_address: sender_address.to_felt(),
                }),
                DeclareTransaction::V2(DeclareTransactionV2 {
                    max_fee,
                    signature,
                    nonce,
                    class_hash,
                    sender_address,
                    compiled_class_hash,
                    ..
                }) => starknet_core::types::DeclareTransaction::V2(starknet_core::types::DeclareTransactionV2 {
                    transaction_hash,
                    max_fee: Felt::from(max_fee.0),
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    nonce: nonce.to_felt(),
                    class_hash: class_hash.to_felt(),
                    sender_address: sender_address.to_felt(),
                    compiled_class_hash: compiled_class_hash.0.to_felt(),
                }),
                DeclareTransaction::V3(DeclareTransactionV3 {
                    resource_bounds,
                    tip,
                    signature,
                    nonce,
                    class_hash,
                    compiled_class_hash,
                    sender_address,
                    nonce_data_availability_mode,
                    fee_data_availability_mode,
                    paymaster_data,
                    account_deployment_data,
                }) => starknet_core::types::DeclareTransaction::V3(starknet_core::types::DeclareTransactionV3 {
                    transaction_hash,
                    resource_bounds: api_resources_to_core_ressources(resource_bounds),
                    tip: tip.0,
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    nonce: nonce.to_felt(),
                    class_hash: class_hash.to_felt(),
                    compiled_class_hash: compiled_class_hash.0.to_felt(),
                    sender_address: sender_address.to_felt(),
                    nonce_data_availability_mode: api_da_to_core_da(nonce_data_availability_mode).unwrap(),
                    fee_data_availability_mode: api_da_to_core_da(fee_data_availability_mode).unwrap(),
                    paymaster_data: paymaster_data.0.iter().map(|x| x.to_felt()).collect(),
                    account_deployment_data: account_deployment_data.0.iter().map(|x| x.to_felt()).collect(),
                }),
            };

            starknet_core::types::Transaction::Declare(tx)
        }
        Transaction::DeployAccount(tx) => {
            let tx = match tx {
                DeployAccountTransaction::V1(DeployAccountTransactionV1 {
                    max_fee,
                    signature,
                    nonce,
                    contract_address_salt,
                    constructor_calldata,
                    class_hash,
                    ..
                }) => starknet_core::types::DeployAccountTransaction::V1(
                    starknet_core::types::DeployAccountTransactionV1 {
                        transaction_hash,
                        max_fee: Felt::from(max_fee.0),
                        signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                        nonce: nonce.to_felt(),
                        contract_address_salt: contract_address_salt.0.to_felt(),
                        constructor_calldata: constructor_calldata.0.iter().map(|x| x.to_felt()).collect(),
                        class_hash: class_hash.to_felt(),
                    },
                ),
                DeployAccountTransaction::V3(DeployAccountTransactionV3 {
                    resource_bounds,
                    tip,
                    signature,
                    nonce,
                    class_hash,
                    contract_address_salt,
                    constructor_calldata,
                    nonce_data_availability_mode,
                    fee_data_availability_mode,
                    paymaster_data,
                }) => starknet_core::types::DeployAccountTransaction::V3(
                    starknet_core::types::DeployAccountTransactionV3 {
                        transaction_hash,
                        resource_bounds: api_resources_to_core_ressources(resource_bounds),
                        tip: tip.0,
                        signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                        nonce: nonce.to_felt(),
                        class_hash: class_hash.to_felt(),
                        contract_address_salt: contract_address_salt.0.to_felt(),
                        constructor_calldata: constructor_calldata.0.iter().map(|x| x.to_felt()).collect(),
                        nonce_data_availability_mode: api_da_to_core_da(nonce_data_availability_mode).unwrap(),
                        fee_data_availability_mode: api_da_to_core_da(fee_data_availability_mode).unwrap(),
                        paymaster_data: paymaster_data.0.iter().map(|x| x.to_felt()).collect(),
                    },
                ),
            };

            starknet_core::types::Transaction::DeployAccount(tx)
        }
        Transaction::Deploy(tx) => {
            let tx = starknet_core::types::DeployTransaction {
                transaction_hash,
                contract_address_salt: tx.contract_address_salt.0.to_felt(),
                constructor_calldata: tx.constructor_calldata.0.iter().map(|x| x.to_felt()).collect(),
                class_hash: tx.class_hash.to_felt(),
                version: Felt::ZERO,
            };

            starknet_core::types::Transaction::Deploy(tx)
        }
        Transaction::Invoke(tx) => {
            let tx = match tx {
                InvokeTransaction::V0(InvokeTransactionV0 {
                    max_fee,
                    signature,
                    contract_address,
                    entry_point_selector,
                    calldata,
                }) => starknet_core::types::InvokeTransaction::V0(starknet_core::types::InvokeTransactionV0 {
                    transaction_hash,
                    max_fee: Felt::from(max_fee.0),
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    contract_address: contract_address.to_felt(),
                    entry_point_selector: entry_point_selector.0.to_felt(),
                    calldata: calldata.0.iter().map(|x| x.to_felt()).collect(),
                }),
                InvokeTransaction::V1(InvokeTransactionV1 {
                    max_fee,
                    signature,
                    nonce,
                    sender_address,
                    calldata,
                    ..
                }) => starknet_core::types::InvokeTransaction::V1(starknet_core::types::InvokeTransactionV1 {
                    transaction_hash,
                    max_fee: Felt::from(max_fee.0),
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    nonce: nonce.to_felt(),
                    sender_address: sender_address.to_felt(),
                    calldata: calldata.0.iter().map(|x| x.to_felt()).collect(),
                }),
                InvokeTransaction::V3(InvokeTransactionV3 {
                    resource_bounds,
                    tip,
                    signature,
                    nonce,
                    sender_address,
                    calldata,
                    nonce_data_availability_mode,
                    fee_data_availability_mode,
                    paymaster_data,
                    account_deployment_data,
                }) => starknet_core::types::InvokeTransaction::V3(starknet_core::types::InvokeTransactionV3 {
                    transaction_hash,
                    resource_bounds: api_resources_to_core_ressources(resource_bounds),
                    tip: tip.0,
                    signature: signature.0.iter().map(|x| x.to_felt()).collect(),
                    nonce: nonce.to_felt(),
                    sender_address: sender_address.to_felt(),
                    calldata: calldata.0.iter().map(|x| x.to_felt()).collect(),
                    nonce_data_availability_mode: api_da_to_core_da(nonce_data_availability_mode).unwrap(),
                    fee_data_availability_mode: api_da_to_core_da(fee_data_availability_mode).unwrap(),
                    paymaster_data: paymaster_data.0.iter().map(|x| x.to_felt()).collect(),
                    account_deployment_data: account_deployment_data.0.iter().map(|x| x.to_felt()).collect(),
                }),
            };

            starknet_core::types::Transaction::Invoke(tx)
        }
        Transaction::L1Handler(tx) => {
            let tx = starknet_core::types::L1HandlerTransaction {
                transaction_hash,
                version: Felt::ZERO,
                nonce: u64::try_from(tx.nonce.0).unwrap(),
                contract_address: tx.contract_address.to_felt(),
                entry_point_selector: tx.entry_point_selector.0.to_felt(),
                calldata: tx.calldata.0.iter().map(|x| x.to_felt()).collect(),
            };

            starknet_core::types::Transaction::L1Handler(tx)
        }
    }
}

// TODO (Tbelleng): Custom function here so check if value are correct
pub fn api_resources_to_core_ressources(resource: &ResourceBoundsMapping) -> CoreResourceBoundsMapping {
    let l1_gas = resource.0.get(&Resource::L1Gas).unwrap();

    let l2_gas = resource.0.get(&Resource::L2Gas).unwrap();

    let resource_for_l1: starknet_core::types::ResourceBounds =
        ResourceBounds { max_amount: l1_gas.max_amount, max_price_per_unit: l1_gas.max_price_per_unit };

    let resource_for_l2: starknet_core::types::ResourceBounds =
        ResourceBounds { max_amount: l2_gas.max_amount, max_price_per_unit: l2_gas.max_price_per_unit };

    CoreResourceBoundsMapping { l1_gas: resource_for_l1, l2_gas: resource_for_l2 }
}

pub fn api_da_to_core_da(
    mode: &starknet_api::data_availability::DataAvailabilityMode,
) -> Option<starknet_core::types::DataAvailabilityMode> {
    match mode {
        starknet_api::data_availability::DataAvailabilityMode::L1 => {
            Some(starknet_core::types::DataAvailabilityMode::L1)
        }
        starknet_api::data_availability::DataAvailabilityMode::L2 => {
            Some(starknet_core::types::DataAvailabilityMode::L2)
        }
    }
}
