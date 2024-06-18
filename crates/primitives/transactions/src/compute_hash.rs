use starknet_api::core::calculate_contract_address;
use starknet_api::data_availability::DataAvailabilityMode;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{
    AccountDeploymentData, Calldata, DeclareTransaction, DeclareTransactionV0V1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, PaymasterData, Resource, ResourceBoundsMapping, Tip, Transaction, TransactionHash,
};

use starknet_core::utils::starknet_keccak;

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash}; //, Poseidon};

use super::SIMULATE_TX_VERSION_OFFSET;
use crate::{LEGACY_BLOCK_NUMBER, LEGACY_L1_HANDLER_BLOCK};
use dp_convert::core_felt::CoreFelt;

const DECLARE_PREFIX: &[u8] = b"declare";
const DEPLOY_ACCOUNT_PREFIX: &[u8] = b"deploy_account";
const DEPLOY_PREFIX: &[u8] = b"deploy";
const INVOKE_PREFIX: &[u8] = b"invoke";
const L1_HANDLER_PREFIX: &[u8] = b"l1_handler";
const L1_GAS: &[u8] = b"L1_GAS";
const L2_GAS: &[u8] = b"L2_GAS";

pub trait ComputeTransactionHash {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash;
}

#[inline]
fn convert_calldata(calldata: Calldata) -> Vec<Felt> {
    calldata.0.iter().map(CoreFelt::into_core_felt).collect()
}

#[inline]
fn compute_calldata_hash_pedersen(calldata: Calldata) -> Felt {
    Pedersen::hash_array(convert_calldata(calldata).as_slice())
}

#[inline]
fn compute_calldata_hash_poseidon(calldata: Calldata) -> Felt {
    Poseidon::hash_array(convert_calldata(calldata).as_slice())
}

#[inline]
fn compute_gas_hash(tip: &Tip, resource_bounds: &ResourceBoundsMapping) -> Felt {
    let gas_as_felt = &[
        Felt::from(tip.0),
        prepare_resource_bound_value(resource_bounds, Resource::L1Gas),
        prepare_resource_bound_value(resource_bounds, Resource::L2Gas),
    ];
    Poseidon::hash_array(gas_as_felt)
}

#[inline]
fn compute_paymaster_hash(paymaster_data: &PaymasterData) -> Felt {
    let paymaster_tmp = paymaster_data.0.iter().map(CoreFelt::into_core_felt).collect::<Vec<_>>();
    Poseidon::hash_array(paymaster_tmp.as_slice())
}

#[inline]
fn compute_account_deployment_hash(account_deployment_data: &AccountDeploymentData) -> Felt {
    let account_deployment_data_tmp =
        &account_deployment_data.0.iter().map(CoreFelt::into_core_felt).collect::<Vec<_>>();
    Poseidon::hash_array(account_deployment_data_tmp.as_slice())
}

// Use a mapping from execution resources to get corresponding fee bounds
// Encodes this information into 32-byte buffer then converts it into Felt
fn prepare_resource_bound_value(resource_bounds_mapping: &ResourceBoundsMapping, resource: Resource) -> Felt {
    let mut buffer = [0u8; 32];
    buffer[2..8].copy_from_slice(match resource {
        Resource::L1Gas => L1_GAS,
        Resource::L2Gas => L2_GAS,
    });
    if let Some(resource_bounds) = resource_bounds_mapping.0.get(&resource) {
        buffer[8..16].copy_from_slice(&resource_bounds.max_amount.to_be_bytes());
        buffer[16..].copy_from_slice(&resource_bounds.max_price_per_unit.to_be_bytes());
    };

    Felt::from_bytes_be(&buffer)
}

fn prepare_data_availability_modes(
    nonce_data_availability_mode: DataAvailabilityMode,
    fee_data_availability_mode: DataAvailabilityMode,
) -> Felt {
    let mut buffer = [0u8; 32];
    buffer[8..12].copy_from_slice(&(nonce_data_availability_mode as u32).to_be_bytes());
    buffer[12..16].copy_from_slice(&(fee_data_availability_mode as u32).to_be_bytes());

    Felt::from_bytes_be(&buffer)
}

impl ComputeTransactionHash for Transaction {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash {
        match self {
            Transaction::Declare(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            Transaction::Deploy(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            Transaction::DeployAccount(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            Transaction::Invoke(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            Transaction::L1Handler(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl ComputeTransactionHash for InvokeTransactionV0 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let sender_address = self.contract_address.into_core_felt();
        let entrypoint_selector = self.entry_point_selector.into_core_felt();

        let calldata_hash = compute_calldata_hash_pedersen(self.calldata.clone());
        let max_fee = Felt::from(self.max_fee.0);

        // Check for deprecated environment
        if block_number > Some(LEGACY_BLOCK_NUMBER) {
            TransactionHash(StarkFelt::new_unchecked(
                Pedersen::hash_array(&[
                    prefix,
                    version,
                    sender_address,
                    entrypoint_selector,
                    calldata_hash,
                    max_fee,
                    chain_id,
                ])
                .to_bytes_be(),
            ))
        } else {
            TransactionHash(StarkFelt::new_unchecked(
                Pedersen::hash_array(&[prefix, sender_address, entrypoint_selector, calldata_hash, chain_id])
                    .to_bytes_be(),
            ))
        }
    }
}

impl ComputeTransactionHash for InvokeTransactionV1 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };
        let sender_address = self.sender_address.into_core_felt();
        let entrypoint_selector = Felt::ZERO;

        let calldata_hash = compute_calldata_hash_pedersen(self.calldata.clone());

        let max_fee = Felt::from(self.max_fee.0);
        let nonce = self.nonce.into_core_felt();

        TransactionHash(StarkFelt::new_unchecked(
            Pedersen::hash_array(&[
                prefix,
                version,
                sender_address,
                entrypoint_selector,
                calldata_hash,
                max_fee,
                chain_id,
                nonce,
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for InvokeTransactionV3 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };
        let sender_address = self.sender_address.into_core_felt();

        let gas_hash = compute_gas_hash(&self.tip, &self.resource_bounds);
        let paymaster_hash = compute_paymaster_hash(&self.paymaster_data);

        let nonce = self.nonce.into_core_felt();
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);

        let account_deployment_data_hash = compute_account_deployment_hash(&self.account_deployment_data);
        let calldata_hash = compute_calldata_hash_poseidon(self.calldata.clone());

        TransactionHash(StarkFelt::new_unchecked(
            Poseidon::hash_array(&[
                prefix,
                version,
                sender_address,
                gas_hash,
                paymaster_hash,
                chain_id,
                nonce,
                data_availability_modes,
                account_deployment_data_hash,
                calldata_hash,
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for InvokeTransaction {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash {
        match self {
            InvokeTransaction::V0(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            InvokeTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            InvokeTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl ComputeTransactionHash for DeclareTransactionV0V1 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(DECLARE_PREFIX);
        let version = if offset_version {
            SIMULATE_TX_VERSION_OFFSET
        } else {
            match block_number {
                Some(0) | None => Felt::ZERO,
                _ => Felt::ONE,
            }
        };

        let sender_address = self.sender_address.into_core_felt();
        let entrypoint_selector = Felt::ZERO;
        let max_fee = Felt::from(self.max_fee.0);

        let class_hash_as_felt = self.class_hash.into_core_felt();

        let nonce_or_class_hash: Felt =
            if version == Felt::ZERO { class_hash_as_felt } else { self.nonce.into_core_felt() };

        let class_or_nothing_hash =
            if version == Felt::ZERO { Pedersen::hash_array(&[]) } else { Pedersen::hash_array(&[class_hash_as_felt]) };

        TransactionHash(StarkFelt::new_unchecked(
            Pedersen::hash_array(&[
                prefix,
                version,
                sender_address,
                entrypoint_selector,
                class_or_nothing_hash,
                max_fee,
                chain_id,
                nonce_or_class_hash,
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for DeclareTransactionV2 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(DECLARE_PREFIX);

        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::TWO } else { Felt::TWO };
        let sender_address = self.sender_address.into_core_felt();
        let entrypoint_selector = Felt::ZERO;

        let calldata = Pedersen::hash_array(&[self.class_hash.into_core_felt()]);

        let max_fee = Felt::from(self.max_fee.0);
        let nonce = self.nonce.into_core_felt();
        let compiled_class_hash = self.compiled_class_hash.into_core_felt();

        TransactionHash(StarkFelt::new_unchecked(
            Pedersen::hash_array(&[
                prefix,
                version,
                sender_address,
                entrypoint_selector,
                calldata,
                max_fee,
                chain_id,
                nonce,
                compiled_class_hash,
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for DeclareTransactionV3 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(DECLARE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };
        let sender_address = self.sender_address.into_core_felt();

        let gas_hash = compute_gas_hash(&self.tip, &self.resource_bounds);
        let paymaster_hash = compute_paymaster_hash(&self.paymaster_data);

        let nonce = self.nonce.into_core_felt();
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);

        let account_deployment_data_hash = compute_account_deployment_hash(&self.account_deployment_data);

        TransactionHash(StarkFelt::new_unchecked(
            Poseidon::hash_array(&[
                prefix,
                version,
                sender_address,
                gas_hash,
                paymaster_hash,
                chain_id,
                nonce,
                data_availability_modes,
                account_deployment_data_hash,
                self.class_hash.into_core_felt(),
                self.compiled_class_hash.into_core_felt(),
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for DeclareTransaction {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        match self {
            // Provisory setting block_number to 0 for V0 to differentiate them from V1 (1)
            DeclareTransaction::V0(tx) => tx.compute_hash(chain_id, offset_version, Some(0)),
            DeclareTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version, Some(1)),
            DeclareTransaction::V2(tx) => tx.compute_hash(chain_id, offset_version, _block_number),
            DeclareTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version, _block_number),
        }
    }
}

impl ComputeTransactionHash for DeployAccountTransaction {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash {
        match self {
            DeployAccountTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            DeployAccountTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl ComputeTransactionHash for DeployAccountTransactionV1 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        let constructor_calldata = convert_calldata(self.constructor_calldata.clone());

        let contract_address = calculate_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Default::default(),
        )
        .unwrap()
        .into_core_felt();

        let prefix = Felt::from_bytes_be_slice(DEPLOY_ACCOUNT_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };
        let entrypoint_selector = Felt::ZERO;

        let mut calldata: Vec<Felt> = Vec::with_capacity(constructor_calldata.len() + 2);
        calldata.push(self.class_hash.into_core_felt());
        calldata.push(self.contract_address_salt.into_core_felt());
        calldata.extend_from_slice(&constructor_calldata);
        let calldata_hash = Pedersen::hash_array(calldata.as_slice());

        let max_fee = Felt::from(self.max_fee.0);
        let nonce = self.nonce.into_core_felt();

        TransactionHash(StarkFelt::new_unchecked(
            Pedersen::hash_array(&[
                prefix,
                version,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                max_fee,
                chain_id,
                nonce,
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for DeployTransaction {
    fn compute_hash(&self, chain_id: Felt, is_query: bool, block_number: Option<u64>) -> TransactionHash {
        let constructor_calldata = convert_calldata(self.constructor_calldata.clone());

        let contract_address = calculate_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Default::default(),
        )
        .unwrap()
        .into_core_felt();

        compute_hash_given_contract_address(
            self.clone(),
            chain_id,
            contract_address,
            is_query,
            block_number,
            &constructor_calldata,
        )
    }
}

impl ComputeTransactionHash for DeployAccountTransactionV3 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(DEPLOY_ACCOUNT_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };

        let constructor_calldata = convert_calldata(self.constructor_calldata.clone());
        let contract_address = calculate_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Default::default(),
        )
        .unwrap()
        .into_core_felt();

        let gas_hash = compute_gas_hash(&self.tip, &self.resource_bounds);
        let paymaster_hash = compute_paymaster_hash(&self.paymaster_data);

        let nonce = self.nonce.into_core_felt();
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);

        let constructor_calldata_hash = Poseidon::hash_array(constructor_calldata.as_slice());

        TransactionHash(StarkFelt::new_unchecked(
            Poseidon::hash_array(&[
                prefix,
                version,
                contract_address,
                gas_hash,
                paymaster_hash,
                chain_id,
                nonce,
                data_availability_modes,
                constructor_calldata_hash,
                self.class_hash.into_core_felt(),
                self.contract_address_salt.into_core_felt(),
            ])
            .to_bytes_be(),
        ))
    }
}

impl ComputeTransactionHash for L1HandlerTransaction {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(L1_HANDLER_PREFIX);
        let invoke_prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let contract_address = self.contract_address.into_core_felt();
        let entrypoint_selector = self.entry_point_selector.into_core_felt();

        let calldata_tmp = convert_calldata(self.calldata.clone());
        let calldata_tdp_bis = calldata_tmp.as_slice();
        let calldata_hash = Pedersen::hash_array(calldata_tdp_bis);

        let nonce = self.nonce.into_core_felt();

        if block_number < Some(LEGACY_L1_HANDLER_BLOCK) && block_number.is_some() {
            TransactionHash(StarkFelt::new_unchecked(
                Pedersen::hash_array(&[invoke_prefix, contract_address, entrypoint_selector, calldata_hash, chain_id])
                    .to_bytes_be(),
            ))
        } else if block_number < Some(LEGACY_BLOCK_NUMBER) && block_number.is_some() {
            TransactionHash(StarkFelt::new_unchecked(
                Pedersen::hash_array(&[prefix, contract_address, entrypoint_selector, calldata_hash, chain_id, nonce])
                    .to_bytes_be(),
            ))
        } else {
            TransactionHash(StarkFelt::new_unchecked(
                Pedersen::hash_array(&[
                    prefix,
                    version,
                    contract_address,
                    entrypoint_selector,
                    calldata_hash,
                    Felt::ZERO, // Fees are set to zero on L1 Handler txs
                    chain_id,
                    nonce,
                ])
                .to_bytes_be(),
            ))
        }
    }
}

pub fn compute_hash_given_contract_address(
    transaction: DeployTransaction,
    chain_id: Felt,
    contract_address: Felt,
    _is_query: bool,
    block_number: Option<u64>,
    constructor_calldata: &[Felt],
) -> TransactionHash {
    let prefix = Felt::from_bytes_be_slice(DEPLOY_PREFIX);
    let version = transaction.version.into_core_felt();

    let constructor_calldata = Pedersen::hash_array(constructor_calldata);

    let constructor = Felt::from_bytes_be(&starknet_keccak(b"constructor").to_bytes_be());

    if block_number > Some(LEGACY_BLOCK_NUMBER) {
        TransactionHash(StarkFelt::new_unchecked(
            Pedersen::hash_array(&[
                prefix,
                version,
                contract_address,
                constructor,
                constructor_calldata,
                Felt::ZERO,
                chain_id,
            ])
            .to_bytes_be(),
        ))
    } else {
        TransactionHash(StarkFelt::new_unchecked(
            Pedersen::hash_array(&[prefix, contract_address, constructor, constructor_calldata, chain_id])
                .to_bytes_be(),
        ))
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    // #[test]
    // fn test() {
    // }
}
