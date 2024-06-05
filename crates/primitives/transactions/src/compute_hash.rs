// use std::intrinsics::mir::Field;

use alloc::vec::Vec;

use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use starknet_api::core::calculate_contract_address;
use starknet_api::data_availability::DataAvailabilityMode;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{
    Calldata, DeclareTransaction, DeclareTransactionV0V1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction, Resource,
    ResourceBoundsMapping, Transaction, TransactionHash,
};
use starknet_core::crypto::compute_hash_on_elements;
use starknet_core::utils::starknet_keccak;
use starknet_crypto::FieldElement;

use starknet_types_core::hash::{Pedersen, StarkHash}; //, Poseidon};
use starknet_types_core::felt::Felt;

use super::SIMULATE_TX_VERSION_OFFSET;
use crate::{LEGACY_BLOCK_NUMBER, LEGACY_L1_HANDLER_BLOCK, SIMULATE_TX_VERSION_OFFSET_FELT};

const DECLARE_PREFIX: &[u8] = b"declare";
const DEPLOY_ACCOUNT_PREFIX: &[u8] = b"deploy_account";
const DEPLOY_PREFIX: &[u8] = b"deploy";
const INVOKE_PREFIX: &[u8] = b"invoke";
const L1_HANDLER_PREFIX: &[u8] = b"l1_handler";
const L1_GAS: &[u8] = b"L1_GAS";
const L2_GAS: &[u8] = b"L2_GAS";

pub trait ComputeTransactionHash {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash;
}

fn convert_calldata(calldata: Calldata) -> Vec<FieldElement> {
    calldata.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect()
}

fn convert_calldata_as_felt(calldata: Calldata) -> Vec<Felt> {
    calldata.0.iter().map(|f| Felt::from_bytes_be(&f.0)).collect()
}

// Use a mapping from execution resources to get corresponding fee bounds
// Encodes this information into 32-byte buffer then converts it into FieldElement
fn prepare_resource_bound_value(resource_bounds_mapping: &ResourceBoundsMapping, resource: Resource) -> FieldElement {
    let mut buffer = [0u8; 32];
    buffer[2..8].copy_from_slice(match resource {
        Resource::L1Gas => L1_GAS,
        Resource::L2Gas => L2_GAS,
    });
    if let Some(resource_bounds) = resource_bounds_mapping.0.get(&resource) {
        buffer[8..16].copy_from_slice(&resource_bounds.max_amount.to_be_bytes());
        buffer[16..].copy_from_slice(&resource_bounds.max_price_per_unit.to_be_bytes());
    };

    // Safe to unwrap because we left most significant bit of the buffer empty
    FieldElement::from_bytes_be(&buffer).unwrap()
}

// Use a mapping from execution resources to get corresponding fee bounds
// Encodes this information into 32-byte buffer then converts it into FieldElement
fn prepare_resource_bound_value_as_felt(resource_bounds_mapping: &ResourceBoundsMapping, resource: Resource) -> Felt {
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
) -> FieldElement {
    let mut buffer = [0u8; 32];
    buffer[8..12].copy_from_slice(&(nonce_data_availability_mode as u32).to_be_bytes());
    buffer[12..16].copy_from_slice(&(fee_data_availability_mode as u32).to_be_bytes());

    FieldElement::from_bytes_be(&buffer).unwrap()
}

fn prepare_data_availability_modes_as_felt(
    nonce_data_availability_mode: DataAvailabilityMode,
    fee_data_availability_mode: DataAvailabilityMode,
) -> Felt {
    let mut buffer = [0u8; 32];
    buffer[8..12].copy_from_slice(&(nonce_data_availability_mode as u32).to_be_bytes());
    buffer[12..16].copy_from_slice(&(fee_data_availability_mode as u32).to_be_bytes());

    Felt::from_bytes_be(&buffer)
}

impl ComputeTransactionHash for Transaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        match self {
            Transaction::Declare(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            Transaction::Deploy(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            Transaction::DeployAccount(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            Transaction::Invoke(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            Transaction::L1Handler(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
        }
    }
}

impl ComputeTransactionHash for InvokeTransactionV0 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET_FELT } else { Felt::ZERO };
        let sender_address = Felt::from_bytes_be(&self.contract_address.0.0.0);
        let entrypoint_selector = Felt::from_bytes_be(&self.entry_point_selector.0.0);

        let calldata_tmp = convert_calldata_as_felt(self.calldata.clone());
        let calldata_tmp_bis = calldata_tmp.as_slice();
        let calldata_hash = Pedersen::hash_array(calldata_tmp_bis);
        
        let max_fee = Felt::from(self.max_fee.0);

        // Check for deprecated environment
        if block_number > Some(LEGACY_BLOCK_NUMBER) {
            TransactionHash(StarkFelt(Pedersen::hash_array(&[
                prefix,
                version,
                sender_address,
                entrypoint_selector,
                calldata_hash,
                max_fee,
                chain_id.into(),
            ]).to_bytes_be()))
        } else {
            TransactionHash(StarkFelt(Pedersen::hash_array(&[
                prefix,
                sender_address,
                entrypoint_selector,
                calldata_hash,
                chain_id.into(),
            ]).to_bytes_be()))
        }
    }
}

impl ComputeTransactionHash for InvokeTransactionV1 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET_FELT + Felt::ONE } else { Felt::ONE };
        let sender_address  = Felt::from_bytes_be(&self.sender_address.0.0.0);
        let entrypoint_selector = Felt::ZERO;

        let calldata_tmp = convert_calldata_as_felt(self.calldata.clone());
        let calldata_tmp_bis = calldata_tmp.as_slice();
        let calldata_hash = Pedersen::hash_array(calldata_tmp_bis);
        
        let max_fee = Felt::from(self.max_fee.0);
        let nonce = Felt::from_bytes_be(&self.nonce.0.0);

        TransactionHash(StarkFelt(Pedersen::hash_array(&[
            prefix,
            version,
            sender_address,
            entrypoint_selector,
            calldata_hash,
            max_fee,
            chain_id.into(),
            nonce,
        ]).to_bytes_be()))
    }
}

impl ComputeTransactionHash for InvokeTransactionV3 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(INVOKE_PREFIX);
        let version =
            if offset_version { SIMULATE_TX_VERSION_OFFSET_FELT + Felt::THREE } else { Felt::THREE };
        let sender_address = Felt::from_bytes_be(&self.sender_address.0.0.0);

        // self.tip.0 is a u64
        let felt_tmp_a = Felt::try_from(self.tip.0).unwrap();
        let felt_tmp_b = prepare_resource_bound_value_as_felt(&self.resource_bounds, Resource::L1Gas);
        let felt_tmp_c = prepare_resource_bound_value_as_felt(&self.resource_bounds, Resource::L2Gas);
        let gas_as_felt = &[felt_tmp_a, felt_tmp_b, felt_tmp_c];
        let gas_hash = Pedersen::hash_array(gas_as_felt);

        let paymaster_tmp = 
            &self.paymaster_data.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>();
        let paymaster_tmp_bis = paymaster_tmp.as_slice();
        let paymaster_hash = Pedersen::hash_array(paymaster_tmp_bis);

        let nonce = Felt::from_bytes_be(&self.nonce.0.0);
        let data_availability_modes = prepare_data_availability_modes_as_felt(
                self.nonce_data_availability_mode, self.fee_data_availability_mode
            );
        
        let data_hash = {
            let account_deployment_data_tmp = 
                &self.account_deployment_data.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>();
            let account_deployment_data_tmp_bis = account_deployment_data_tmp.as_slice();
            let account_deployment_data_hash = Pedersen::hash_array(account_deployment_data_tmp_bis);


            let calldata_tmp = convert_calldata_as_felt(self.calldata.clone());
            let calldata_tmp_bis = calldata_tmp.as_slice();
            let calldata_hash = Pedersen::hash_array(calldata_tmp_bis);    

            Pedersen::hash_array(&[account_deployment_data_hash, calldata_hash])
        };

        TransactionHash(StarkFelt(Pedersen::hash_array(&[
            prefix,
            version,
            sender_address,
            gas_hash,
            paymaster_hash,
            chain_id.into(),
            nonce,
            data_availability_modes,
            data_hash,
            ]).to_bytes_be()))
    }
}

impl ComputeTransactionHash for InvokeTransaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        match self {
            InvokeTransaction::V0(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            InvokeTransaction::V1(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            InvokeTransaction::V3(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
        }
    }
}

impl ComputeTransactionHash for DeclareTransactionV0V1 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(DECLARE_PREFIX);
        let version = if offset_version {
            SIMULATE_TX_VERSION_OFFSET_FELT
        } else {
            match block_number {
                Some(0) | None => Felt::ZERO,
                _ => Felt::ONE,
            }
        };
        
        let sender_address = Felt::from_bytes_be(&self.sender_address.0.0.0);
        let entrypoint_selector = Felt::ZERO;
        let max_fee = Felt::from(self.max_fee.0);
        
        let class_hash_as_felt = Felt::from_bytes_be(&self.class_hash.0.0);
        
        let nonce_or_class_hash: Felt = if version == Felt::ZERO {
            class_hash_as_felt
        } else {
            Felt::from_bytes_be(&self.nonce.0.0)
        };
        
        let class_or_nothing_hash = if version == Felt::ZERO {
            Pedersen::hash_array(&[])
        } else {
            Pedersen::hash_array(&[class_hash_as_felt])
        };

        TransactionHash(StarkFelt(Pedersen::hash_array(&[
            prefix,
            version,
            sender_address,
            entrypoint_selector,
            class_or_nothing_hash,
            max_fee,
            chain_id.into(),
            nonce_or_class_hash,
            ]).to_bytes_be()))
    }
}

impl ComputeTransactionHash for DeclareTransactionV2 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = Felt::from_bytes_be_slice(DECLARE_PREFIX);

        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET_FELT + Felt::TWO } else { Felt::TWO };
        let sender_address = Felt::from_bytes_be(&self.sender_address.0.0.0);
        let entrypoint_selector = Felt::ZERO;

        let calldata = Pedersen::hash_array(&[Felt::from_bytes_be(&self.class_hash.0.0)]);
        
        let max_fee = Felt::from(self.max_fee.0);
        let nonce = Felt::from_bytes_be(&self.nonce.0.0);
        let compiled_class_hash = Felt::from_bytes_be(&self.compiled_class_hash.0.0);

        TransactionHash(StarkFelt(Pedersen::hash_array(&[
            prefix,
            version,
            sender_address,
            entrypoint_selector,
            calldata,
            max_fee,
            chain_id.into(),
            nonce,
            compiled_class_hash,
            ]).to_bytes_be()))
    }
}

impl ComputeTransactionHash for DeclareTransactionV3 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(DECLARE_PREFIX).unwrap();
        let version =
            if offset_version { SIMULATE_TX_VERSION_OFFSET + FieldElement::THREE } else { FieldElement::THREE };
        let sender_address = Felt252Wrapper::from(self.sender_address).into();
        let gas_hash = compute_hash_on_elements(&[
            FieldElement::from(self.tip.0),
            prepare_resource_bound_value(&self.resource_bounds, Resource::L1Gas),
            prepare_resource_bound_value(&self.resource_bounds, Resource::L2Gas),
        ]);
        let paymaster_hash = compute_hash_on_elements(
            &self.paymaster_data.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>(),
        );
        let nonce = Felt252Wrapper::from(self.nonce.0).into();
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);
        let account_deployment_data_hash = compute_hash_on_elements(
            &self.account_deployment_data.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>(),
        );

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            sender_address,
            gas_hash,
            paymaster_hash,
            chain_id.into(),
            nonce,
            data_availability_modes,
            account_deployment_data_hash,
            Felt252Wrapper::from(self.class_hash).into(),
            Felt252Wrapper::from(self.compiled_class_hash).into(),
        ]))
        .into()
    }
}

impl ComputeTransactionHash for DeclareTransaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        match self {
            // Provisory setting block_number to 0 for V0 to differentiate them from V1 (1)
            DeclareTransaction::V0(tx) => tx.compute_hash::<H>(chain_id, offset_version, Some(0)),
            DeclareTransaction::V1(tx) => tx.compute_hash::<H>(chain_id, offset_version, Some(1)),
            DeclareTransaction::V2(tx) => tx.compute_hash::<H>(chain_id, offset_version, _block_number),
            DeclareTransaction::V3(tx) => tx.compute_hash::<H>(chain_id, offset_version, _block_number),
        }
    }
}

impl ComputeTransactionHash for DeployAccountTransaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        match self {
            DeployAccountTransaction::V1(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
            DeployAccountTransaction::V3(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
        }
    }
}

impl ComputeTransactionHash for DeployAccountTransactionV1 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let constructor_calldata = convert_calldata(self.constructor_calldata.clone());

        let contract_address = Felt252Wrapper::from(
            calculate_contract_address(
                self.contract_address_salt,
                self.class_hash,
                &self.constructor_calldata,
                Default::default(),
            )
            .unwrap(),
        )
        .into();
        let prefix = FieldElement::from_byte_slice_be(DEPLOY_ACCOUNT_PREFIX).unwrap();
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + FieldElement::ONE } else { FieldElement::ONE };
        let entrypoint_selector = FieldElement::ZERO;
        let mut calldata: Vec<FieldElement> = Vec::with_capacity(constructor_calldata.len() + 2);
        calldata.push(Felt252Wrapper::from(self.class_hash).into());
        calldata.push(Felt252Wrapper::from(self.contract_address_salt).into());
        calldata.extend_from_slice(&constructor_calldata);
        let calldata_hash = compute_hash_on_elements(&calldata);
        let max_fee = FieldElement::from(self.max_fee.0);
        let nonce = Felt252Wrapper::from(self.nonce).into();

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            contract_address,
            entrypoint_selector,
            calldata_hash,
            max_fee,
            chain_id.into(),
            nonce,
        ]))
        .into()
    }
}

impl ComputeTransactionHash for DeployTransaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        is_query: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        let chain_id = chain_id.into();
        let constructor_calldata = convert_calldata(self.constructor_calldata.clone());

        let contract_address = Felt252Wrapper::from(
            calculate_contract_address(
                self.contract_address_salt,
                self.class_hash,
                &self.constructor_calldata,
                Default::default(),
            )
            .unwrap(),
        )
        .into();

        compute_hash_given_contract_address::<H>(
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
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(DEPLOY_ACCOUNT_PREFIX).unwrap();
        let version =
            if offset_version { SIMULATE_TX_VERSION_OFFSET + FieldElement::THREE } else { FieldElement::THREE };
        let constructor_calldata = convert_calldata(self.constructor_calldata.clone());
        let contract_address = Felt252Wrapper::from(
            calculate_contract_address(
                self.contract_address_salt,
                self.class_hash,
                &self.constructor_calldata,
                Default::default(),
            )
            .unwrap(),
        )
        .into();
        let gas_hash = compute_hash_on_elements(&[
            FieldElement::from(self.tip.0),
            prepare_resource_bound_value(&self.resource_bounds, Resource::L1Gas),
            prepare_resource_bound_value(&self.resource_bounds, Resource::L2Gas),
        ]);
        let paymaster_hash = compute_hash_on_elements(
            &self.paymaster_data.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>(),
        );
        let nonce = Felt252Wrapper::from(self.nonce.0).into();
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);
        let constructor_calldata_hash = compute_hash_on_elements(&constructor_calldata);

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            contract_address,
            gas_hash,
            paymaster_hash,
            chain_id.into(),
            nonce,
            data_availability_modes,
            constructor_calldata_hash,
            Felt252Wrapper::from(self.class_hash).into(),
            Felt252Wrapper::from(self.contract_address_salt).into(),
        ]))
        .into()
    }
}

impl ComputeTransactionHash for L1HandlerTransaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(L1_HANDLER_PREFIX).unwrap();
        let invoke_prefix = FieldElement::from_byte_slice_be(INVOKE_PREFIX).unwrap();
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { FieldElement::ZERO };
        let contract_address = Felt252Wrapper::from(self.contract_address).into();
        let entrypoint_selector = Felt252Wrapper::from(self.entry_point_selector).into();
        let calldata_hash = compute_hash_on_elements(&convert_calldata(self.calldata.clone()));
        let nonce = Felt252Wrapper::from(self.nonce).into();
        let chain_id = chain_id.into();

        if block_number < Some(LEGACY_L1_HANDLER_BLOCK) && block_number.is_some() {
            Felt252Wrapper::from(H::compute_hash_on_elements(&[
                invoke_prefix,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                chain_id,
            ]))
            .into()
        } else if block_number < Some(LEGACY_BLOCK_NUMBER) && block_number.is_some() {
            Felt252Wrapper::from(H::compute_hash_on_elements(&[
                prefix,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                chain_id,
                nonce,
            ]))
            .into()
        } else {
            Felt252Wrapper::from(H::compute_hash_on_elements(&[
                prefix,
                version,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                FieldElement::ZERO, // Fees are set to zero on L1 Handler txs
                chain_id,
                nonce,
            ]))
            .into()
        }
    }
}

pub fn compute_hash_given_contract_address<H: HasherT>(
    transaction: DeployTransaction,
    chain_id: FieldElement,
    contract_address: FieldElement,
    _is_query: bool,
    block_number: Option<u64>,
    constructor_calldata: &[FieldElement],
) -> TransactionHash {
    let prefix = FieldElement::from_byte_slice_be(DEPLOY_PREFIX).unwrap();
    let version = Felt252Wrapper::from(transaction.version.0).into();
    let constructor_calldata = compute_hash_on_elements(constructor_calldata);

    let constructor = starknet_keccak(b"constructor");

    if block_number > Some(LEGACY_BLOCK_NUMBER) {
        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            contract_address,
            constructor,
            constructor_calldata,
            FieldElement::ZERO,
            chain_id,
        ]))
        .into()
    } else {
        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            contract_address,
            constructor,
            constructor_calldata,
            chain_id,
        ]))
        .into()
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    // #[test]
    // fn test() {
    // }
}