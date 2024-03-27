use alloc::vec::Vec;

use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use starknet_api::core::calculate_contract_address;
use starknet_api::data_availability::DataAvailabilityMode;
use starknet_api::transaction::{
    Calldata, DeclareTransaction, DeclareTransactionV0V1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, InvokeTransaction,
    InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction, Resource,
    ResourceBoundsMapping, TransactionHash,
};
use starknet_core::crypto::compute_hash_on_elements;
use starknet_crypto::FieldElement;
use starknet_core::utils::starknet_keccak;

use super::SIMULATE_TX_VERSION_OFFSET;
use crate::{DeployTransaction, UserOrL1HandlerTransaction, LEGACY_BLOCK_NUMBER};

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

fn prepare_data_availability_modes(
    nonce_data_availability_mode: DataAvailabilityMode,
    fee_data_availability_mode: DataAvailabilityMode,
) -> FieldElement {
    let mut buffer = [0u8; 32];
    buffer[64..96].copy_from_slice(&(nonce_data_availability_mode as u32).to_be_bytes());
    buffer[96..].copy_from_slice(&(fee_data_availability_mode as u32).to_be_bytes());

    // Safe to unwrap because we left most significant bit of the buffer empty
    FieldElement::from_bytes_be(&buffer).unwrap()
}

impl ComputeTransactionHash for InvokeTransactionV0 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(INVOKE_PREFIX).unwrap();
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { FieldElement::ZERO };
        let sender_address = Felt252Wrapper::from(self.contract_address).into();
        let entrypoint_selector = Felt252Wrapper::from(self.entry_point_selector).into();
        let calldata_hash = compute_hash_on_elements(&convert_calldata(self.calldata.clone()));
        let max_fee = FieldElement::from(self.max_fee.0);

        // Check for deprecated environment
        if block_number > Some(LEGACY_BLOCK_NUMBER) {
            Felt252Wrapper(H::compute_hash_on_elements(&[
                prefix,
                version,
                sender_address,
                entrypoint_selector,
                calldata_hash,
                max_fee,
                chain_id.into(),
            ]))
            .into()
        } else {
            Felt252Wrapper(H::compute_hash_on_elements(&[
                prefix,
                sender_address,
                entrypoint_selector,
                calldata_hash,
                chain_id.into(),
            ]))
            .into()
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
        let prefix = FieldElement::from_byte_slice_be(INVOKE_PREFIX).unwrap();
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + FieldElement::ONE } else { FieldElement::ONE };
        let sender_address = Felt252Wrapper::from(self.sender_address).into();
        let entrypoint_selector = FieldElement::ZERO;
        let calldata_hash = compute_hash_on_elements(&convert_calldata(self.calldata.clone()));
        let max_fee = FieldElement::from(self.max_fee.0);
        let nonce = Felt252Wrapper::from(self.nonce.0).into();

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            sender_address,
            entrypoint_selector,
            calldata_hash,
            max_fee,
            chain_id.into(),
            nonce,
        ]))
        .into()
    }
}

impl ComputeTransactionHash for InvokeTransactionV3 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(INVOKE_PREFIX).unwrap();
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
        let data_hash = {
            let account_deployment_data_hash = compute_hash_on_elements(
                &self.account_deployment_data.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>(),
            );
            let calldata_hash = compute_hash_on_elements(
                &self.calldata.0.iter().map(|f| Felt252Wrapper::from(*f).into()).collect::<Vec<_>>(),
            );
            compute_hash_on_elements(&[account_deployment_data_hash, calldata_hash])
        };

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            sender_address,
            gas_hash,
            paymaster_hash,
            chain_id.into(),
            nonce,
            data_availability_modes,
            data_hash,
        ]))
        .into()
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
// TODO: Check this implem, Madara-wip do it using another way, a function insted of an implem
impl ComputeTransactionHash for DeclareTransactionV0V1 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(DECLARE_PREFIX).unwrap();
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { FieldElement::ZERO };
        let sender_address = Felt252Wrapper::from(self.sender_address).into();
        let entrypoint_selector = FieldElement::ZERO;
        let alignment_placeholder = compute_hash_on_elements(&[]);
        let max_fee = FieldElement::from(self.max_fee.0);
        let nonce_or_class_hash: FieldElement = if version == FieldElement::ZERO {
            Felt252Wrapper::from(self.class_hash).into()
        } else {
            Felt252Wrapper::from(self.nonce).into()
        };
        let class_or_nothing_hash = if version == FieldElement::ZERO {
            compute_hash_on_elements(&[])
        } else {
            compute_hash_on_elements(&[Felt252Wrapper::from(self.class_hash).into()])
        };

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            sender_address,
            entrypoint_selector,
            class_or_nothing_hash,
            max_fee,
            chain_id.into(),
            nonce_or_class_hash,
        ]))
        .into()
    }
}

impl ComputeTransactionHash for DeclareTransactionV2 {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        offset_version: bool,
        _block_number: Option<u64>,
    ) -> TransactionHash {
        let prefix = FieldElement::from_byte_slice_be(DECLARE_PREFIX).unwrap();
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + FieldElement::TWO } else { FieldElement::TWO };
        let sender_address = Felt252Wrapper::from(self.sender_address).into();
        let entrypoint_selector = FieldElement::ZERO;
        let calldata = compute_hash_on_elements(&[Felt252Wrapper::from(self.class_hash).into()]);
        let max_fee = FieldElement::from(self.max_fee.0);
        let nonce = Felt252Wrapper::from(self.nonce).into();
        let compiled_class_hash = Felt252Wrapper::from(self.compiled_class_hash).into();

        Felt252Wrapper(H::compute_hash_on_elements(&[
            prefix,
            version,
            sender_address,
            entrypoint_selector,
            calldata,
            max_fee,
            chain_id.into(),
            nonce,
            compiled_class_hash,
        ]))
        .into()
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
            DeclareTransaction::V0(tx) => tx.compute_hash::<H>(chain_id, offset_version, _block_number),
            DeclareTransaction::V1(tx) => tx.compute_hash::<H>(chain_id, offset_version, _block_number),
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

impl ComputeTransactionHash for DeployTransaction {
    fn compute_hash<H: HasherT>(
        &self,
        chain_id: Felt252Wrapper,
        is_query: bool,
        block_number: Option<u64>,
    ) -> TransactionHash {
        let chain_id = chain_id.into();
        let contract_address = self.account_address().into();

        let constructor_calldata = self.constructor_calldata.clone();
        let constructor_calldata_vec: Vec<FieldElement> = constructor_calldata.into_iter().map(|x| x.into()).collect();

        compute_hash_given_contract_address::<H>(self.clone(), chain_id, contract_address, is_query, block_number, &constructor_calldata_vec)
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

        if block_number > Some(LEGACY_BLOCK_NUMBER) && block_number.is_some() {
            Felt252Wrapper(H::compute_hash_on_elements(&[
                invoke_prefix,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                chain_id.into(),
            ]))
            .into()
        } else if block_number < Some(LEGACY_BLOCK_NUMBER) && block_number.is_some() {
            Felt252Wrapper(H::compute_hash_on_elements(&[
                prefix,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                chain_id.into(),
                nonce,
            ]))
            .into()
        } else {
            Felt252Wrapper(H::compute_hash_on_elements(&[
                prefix,
                version,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                FieldElement::ZERO, //fees are set to 0 on l1 handlerTx
                chain_id.into(),
                nonce,
            ]))
            .into()
        }
    }
}

// impl ComputeTransactionHash for UserOrL1HandlerTransaction {
//     fn compute_hash<H: HasherT>(
//         &self,
//         chain_id: Felt252Wrapper,
//         offset_version: bool,
//         block_number: Option<u64>,
//     ) -> TransactionHash {
//         match self {
//             UserOrL1HandlerTransaction::User(tx) => tx.compute_hash::<H>(chain_id, offset_version, block_number),
//             UserOrL1HandlerTransaction::L1Handler(tx, _) => {
//                 tx.compute_hash::<H>(chain_id, offset_version, block_number)
//             }
//         }
//     }
// }

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
        Felt252Wrapper(H::compute_hash_on_elements(&[prefix, contract_address, constructor, constructor_calldata, chain_id])).into()
    }
}

#[cfg(test)]
#[path = "compute_hash_tests.rs"]
mod compute_hash_tests;
