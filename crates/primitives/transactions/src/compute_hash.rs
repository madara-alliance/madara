use std::sync::Arc;

use dp_convert::ToStarkFelt;
use starknet_api::core::{calculate_contract_address, ClassHash};
use starknet_api::transaction::{Calldata, ContractAddressSalt};
use starknet_core::utils::starknet_keccak;

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash}; //, Poseidon};

use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBoundsMapping, Transaction,
};

use super::SIMULATE_TX_VERSION_OFFSET;
use crate::{LEGACY_BLOCK_NUMBER, LEGACY_L1_HANDLER_BLOCK, MAIN_CHAIN_ID};
use dp_convert::ToFelt;

// b"declare" == 0x6465636c617265
const DECLARE_PREFIX: Felt = Felt::from_hex_unchecked("0x6465636c617265");
// b"deploy_account" == 0x6465706c6f795f6163636f756e74
const DEPLOY_ACCOUNT_PREFIX: Felt = Felt::from_hex_unchecked("0x6465706c6f795f6163636f756e74");
// b"deploy" == 0x6465706c6f79
const DEPLOY_PREFIX: Felt = Felt::from_hex_unchecked("0x6465706c6f79");
// b"invoke" == 0x696e766f6b65
const INVOKE_PREFIX: Felt = Felt::from_hex_unchecked("0x696e766f6b65");
// b"l1_handler" == 0x6c315f68616e646c6572
const L1_HANDLER_PREFIX: Felt = Felt::from_hex_unchecked("0x6c315f68616e646c6572");

const L1_GAS: &[u8] = b"L1_GAS";
const L2_GAS: &[u8] = b"L2_GAS";

impl Transaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> Felt {
        match self {
            crate::Transaction::Invoke(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            crate::Transaction::L1Handler(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            crate::Transaction::Declare(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            crate::Transaction::Deploy(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            crate::Transaction::DeployAccount(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl InvokeTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> Felt {
        match self {
            InvokeTransaction::V0(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            InvokeTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            InvokeTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl InvokeTransactionV0 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let sender_address = self.contract_address;
        let entrypoint_selector = self.entry_point_selector;
        let calldata_hash = Pedersen::hash_array(&self.calldata);
        let max_fee = self.max_fee;

        // Check for deprecated environment
        if block_number < Some(LEGACY_BLOCK_NUMBER) && chain_id == MAIN_CHAIN_ID {
            Pedersen::hash_array(&[INVOKE_PREFIX, sender_address, entrypoint_selector, calldata_hash, chain_id])
        } else {
            Pedersen::hash_array(&[
                INVOKE_PREFIX,
                version,
                sender_address,
                entrypoint_selector,
                calldata_hash,
                max_fee,
                chain_id,
            ])
        }
    }
}

impl InvokeTransactionV1 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };
        let sender_address = self.sender_address;
        let entrypoint_selector = Felt::ZERO;
        let calldata_hash = Pedersen::hash_array(&self.calldata);
        let max_fee = self.max_fee;
        let nonce = self.nonce;

        Pedersen::hash_array(&[
            INVOKE_PREFIX,
            version,
            sender_address,
            entrypoint_selector,
            calldata_hash,
            max_fee,
            chain_id,
            nonce,
        ])
    }
}

impl InvokeTransactionV3 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };
        let sender_address = self.sender_address;
        let gas_hash = compute_gas_hash(self.tip, &self.resource_bounds);
        let paymaster_hash = Poseidon::hash_array(&self.paymaster_data);
        let nonce = self.nonce;
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);
        let account_deployment_data_hash = Poseidon::hash_array(&self.account_deployment_data);
        let calldata_hash = Pedersen::hash_array(&self.calldata);

        Poseidon::hash_array(&[
            INVOKE_PREFIX,
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
    }
}

impl L1HandlerTransaction {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let contract_address = self.contract_address;
        let entrypoint_selector = self.entry_point_selector;
        let calldata_hash = Pedersen::hash_array(&self.calldata);
        let nonce = self.nonce.into();

        if block_number < Some(LEGACY_L1_HANDLER_BLOCK) && chain_id == MAIN_CHAIN_ID {
            Pedersen::hash_array(&[INVOKE_PREFIX, contract_address, entrypoint_selector, calldata_hash, chain_id])
        } else if block_number < Some(LEGACY_BLOCK_NUMBER) && chain_id == MAIN_CHAIN_ID {
            Pedersen::hash_array(&[
                L1_HANDLER_PREFIX,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                chain_id,
                nonce,
            ])
        } else {
            Pedersen::hash_array(&[
                L1_HANDLER_PREFIX,
                version,
                contract_address,
                entrypoint_selector,
                calldata_hash,
                Felt::ZERO, // Fees are set to zero on L1 Handler txs
                chain_id,
                nonce,
            ])
        }
    }
}

impl DeclareTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> Felt {
        match self {
            DeclareTransaction::V0(tx) => tx.compute_hash(chain_id, offset_version, None),
            DeclareTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version, None),
            DeclareTransaction::V2(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            DeclareTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl DeclareTransactionV0 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };

        let sender_address = self.sender_address;
        let entrypoint_selector = Felt::ZERO;
        let max_fee = self.max_fee;

        let class_hash = self.class_hash;

        let nonce_or_class_hash = class_hash;

        let class_or_nothing_hash =
            if version == Felt::ZERO { Pedersen::hash_array(&[]) } else { Pedersen::hash_array(&[class_hash]) };

        Pedersen::hash_array(&[
            DECLARE_PREFIX,
            version,
            sender_address,
            entrypoint_selector,
            class_or_nothing_hash,
            max_fee,
            chain_id,
            nonce_or_class_hash,
        ])
    }
}

impl DeclareTransactionV1 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ONE };

        let sender_address = self.sender_address;
        let entrypoint_selector = Felt::ZERO;
        let max_fee = self.max_fee;

        let class_hash = self.class_hash;

        let nonce_or_class_hash = self.nonce;

        let class_or_nothing_hash =
            if version == Felt::ZERO { Pedersen::hash_array(&[]) } else { Pedersen::hash_array(&[class_hash]) };

        Pedersen::hash_array(&[
            DECLARE_PREFIX,
            version,
            sender_address,
            entrypoint_selector,
            class_or_nothing_hash,
            max_fee,
            chain_id,
            nonce_or_class_hash,
        ])
    }
}

impl DeclareTransactionV2 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::TWO } else { Felt::TWO };
        let sender_address = self.sender_address;
        let entrypoint_selector = Felt::ZERO;

        let calldata = Pedersen::hash_array(&[self.class_hash]);

        let max_fee = self.max_fee;
        let nonce = self.nonce;
        let compiled_class_hash = self.compiled_class_hash;

        Pedersen::hash_array(&[
            DECLARE_PREFIX,
            version,
            sender_address,
            entrypoint_selector,
            calldata,
            max_fee,
            chain_id,
            nonce,
            compiled_class_hash,
        ])
    }
}

impl DeclareTransactionV3 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };
        let sender_address = self.sender_address;

        let gas_hash = compute_gas_hash(self.tip, &self.resource_bounds);
        let paymaster_hash = Poseidon::hash_array(&self.paymaster_data);

        let nonce = self.nonce;
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);

        let account_deployment_data_hash = Poseidon::hash_array(&self.account_deployment_data);

        Poseidon::hash_array(&[
            DECLARE_PREFIX,
            version,
            sender_address,
            gas_hash,
            paymaster_hash,
            chain_id,
            nonce,
            data_availability_modes,
            account_deployment_data_hash,
            self.class_hash,
            self.compiled_class_hash,
        ])
    }
}

impl DeployAccountTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, block_number: Option<u64>) -> Felt {
        match self {
            DeployAccountTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version, block_number),
            DeployAccountTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version, block_number),
        }
    }
}

impl DeployAccountTransactionV1 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let contract_address = calculate_contract_address(
            ContractAddressSalt(self.contract_address_salt.to_stark_felt()),
            ClassHash(self.class_hash.to_stark_felt()),
            &Calldata(Arc::new(self.constructor_calldata.iter().map(|x| x.to_stark_felt()).collect::<Vec<_>>())),
            Default::default(),
        )
        .unwrap()
        .to_felt();

        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };
        let entrypoint_selector = Felt::ZERO;

        let mut calldata: Vec<Felt> = Vec::with_capacity(self.constructor_calldata.len() + 2);
        calldata.push(self.class_hash);
        calldata.push(self.contract_address_salt);
        calldata.extend_from_slice(&self.constructor_calldata);
        let calldata_hash = Pedersen::hash_array(calldata.as_slice());

        let max_fee = self.max_fee;
        let nonce = self.nonce;

        Pedersen::hash_array(&[
            DEPLOY_ACCOUNT_PREFIX,
            version,
            contract_address,
            entrypoint_selector,
            calldata_hash,
            max_fee,
            chain_id,
            nonce,
        ])
    }
}

impl DeployAccountTransactionV3 {
    fn compute_hash(&self, chain_id: Felt, offset_version: bool, _block_number: Option<u64>) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };

        let contract_address = calculate_contract_address(
            ContractAddressSalt(self.contract_address_salt.to_stark_felt()),
            ClassHash(self.class_hash.to_stark_felt()),
            &Calldata(Arc::new(self.constructor_calldata.iter().map(|x| x.to_stark_felt()).collect::<Vec<_>>())),
            Default::default(),
        )
        .unwrap()
        .to_felt();

        let gas_hash = compute_gas_hash(self.tip, &self.resource_bounds);
        let paymaster_hash = Poseidon::hash_array(&self.paymaster_data);

        let nonce = self.nonce;
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);

        let constructor_calldata_hash = Poseidon::hash_array(&self.constructor_calldata);
        Poseidon::hash_array(&[
            DEPLOY_ACCOUNT_PREFIX,
            version,
            contract_address,
            gas_hash,
            paymaster_hash,
            chain_id,
            nonce,
            data_availability_modes,
            constructor_calldata_hash,
            self.class_hash,
            self.contract_address_salt,
        ])
    }
}

impl DeployTransaction {
    fn compute_hash(&self, chain_id: Felt, is_query: bool, block_number: Option<u64>) -> Felt {
        let contract_address = calculate_contract_address(
            ContractAddressSalt(self.contract_address_salt.to_stark_felt()),
            ClassHash(self.class_hash.to_stark_felt()),
            &Calldata(Arc::new(self.constructor_calldata.iter().map(|x| x.to_stark_felt()).collect::<Vec<_>>())),
            Default::default(),
        )
        .unwrap()
        .to_felt();

        compute_hash_given_contract_address(
            self,
            chain_id,
            contract_address,
            is_query,
            block_number,
            &self.constructor_calldata,
        )
    }
}

pub fn compute_hash_given_contract_address(
    transaction: &DeployTransaction,
    chain_id: Felt,
    contract_address: Felt,
    _is_query: bool,
    block_number: Option<u64>,
    constructor_calldata: &[Felt],
) -> Felt {
    let version = transaction.version;

    let constructor_calldata = Pedersen::hash_array(constructor_calldata);

    let constructor = Felt::from_bytes_be(&starknet_keccak(b"constructor").to_bytes_be());

    if block_number < Some(LEGACY_BLOCK_NUMBER) && chain_id == MAIN_CHAIN_ID {
        Pedersen::hash_array(&[DEPLOY_PREFIX, contract_address, constructor, constructor_calldata, chain_id])
    } else {
        Pedersen::hash_array(&[
            DEPLOY_PREFIX,
            version,
            contract_address,
            constructor,
            constructor_calldata,
            Felt::ZERO,
            chain_id,
        ])
    }
}

#[inline]
fn compute_gas_hash(tip: u64, resource_bounds: &ResourceBoundsMapping) -> Felt {
    let gas_as_felt = &[
        Felt::from(tip),
        prepare_resource_bound_value(resource_bounds, DataAvailabilityMode::L1),
        prepare_resource_bound_value(resource_bounds, DataAvailabilityMode::L2),
    ];
    Poseidon::hash_array(gas_as_felt)
}

// Use a mapping from execution resources to get corresponding fee bounds
// Encodes this information into 32-byte buffer then converts it into Felt
fn prepare_resource_bound_value(
    resource_bounds_mapping: &ResourceBoundsMapping,
    da_mode: DataAvailabilityMode,
) -> Felt {
    let mut buffer = [0u8; 32];

    buffer[2..8].copy_from_slice(match da_mode {
        DataAvailabilityMode::L1 => L1_GAS,
        DataAvailabilityMode::L2 => L2_GAS,
    });
    let resource_bounds = match da_mode {
        DataAvailabilityMode::L1 => resource_bounds_mapping.l1_gas.clone(),
        DataAvailabilityMode::L2 => resource_bounds_mapping.l2_gas.clone(),
    };
    buffer[8..16].copy_from_slice(&resource_bounds.max_amount.to_be_bytes());
    buffer[16..].copy_from_slice(&resource_bounds.max_price_per_unit.to_be_bytes());

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
