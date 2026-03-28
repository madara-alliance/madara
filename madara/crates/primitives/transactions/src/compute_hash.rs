use mp_chain_config::StarknetVersion;
use starknet_core::utils::starknet_keccak;

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction,
};

use super::SIMULATE_TX_VERSION_OFFSET;

// contants for transaction prefixes
const DECLARE_PREFIX: Felt = Felt::from_hex_unchecked("0x6465636c617265"); // b"declare"
const DEPLOY_ACCOUNT_PREFIX: Felt = Felt::from_hex_unchecked("0x6465706c6f795f6163636f756e74"); // b"deploy_account"
const DEPLOY_PREFIX: Felt = Felt::from_hex_unchecked("0x6465706c6f79"); // b"deploy"
const INVOKE_PREFIX: Felt = Felt::from_hex_unchecked("0x696e766f6b65"); // b"invoke"
const L1_HANDLER_PREFIX: Felt = Felt::from_hex_unchecked("0x6c315f68616e646c6572"); // b"l1_handler"

const PEDERSEN_EMPTY: Felt =
    Felt::from_hex_unchecked("0x49ee3eba8c1600700ee1b87eb599f16716b0b1022947733551fde4050ca6804");

impl Transaction {
    pub fn compute_hash(&self, chain_id: Felt, version: StarknetVersion, is_query: bool) -> Felt {
        let legacy = version.is_legacy();
        let is_pre_v0_7 = version.is_pre_v0_7();

        if is_pre_v0_7 {
            self.compute_hash_pre_v0_7(chain_id, is_query)
        } else {
            self.compute_hash_inner(chain_id, is_query, legacy)
        }
    }

    fn compute_hash_inner(&self, chain_id: Felt, is_query: bool, legacy: bool) -> Felt {
        match self {
            crate::Transaction::Invoke(tx) => tx.compute_hash(chain_id, is_query, legacy),
            crate::Transaction::L1Handler(tx) => tx.compute_hash(chain_id, is_query, legacy),
            crate::Transaction::Declare(tx) => tx.compute_hash(chain_id, is_query),
            crate::Transaction::Deploy(tx) => tx.compute_hash(chain_id, is_query, legacy),
            crate::Transaction::DeployAccount(tx) => tx.compute_hash(chain_id, is_query),
        }
    }

    pub fn compute_hash_pre_v0_7(&self, chain_id: Felt, is_query: bool) -> Felt {
        match self {
            crate::Transaction::L1Handler(tx) => tx.compute_hash_pre_v0_7(chain_id),
            _ => self.compute_hash_inner(chain_id, is_query, true),
        }
    }

    /// Compute the combined hash of the transaction hash and the signature.
    ///
    /// Since the transaction hash doesn't take the signature values as its input
    /// computing the transaction commitent uses a hash value that combines
    /// the transaction hash with the array of signature values.
    pub fn compute_hash_with_signature(&self, tx_hash: Felt, starknet_version: StarknetVersion) -> Felt {
        if starknet_version < StarknetVersion::V0_11_1 {
            self.compute_hash_with_signature_pre_v0_11_1(tx_hash)
        } else if starknet_version < StarknetVersion::V0_13_2 {
            self.compute_hash_with_signature_pre_v0_13_2(tx_hash)
        } else if starknet_version < StarknetVersion::V0_13_4 {
            self.compute_hash_with_signature_pre_v0_13_4(tx_hash)
        } else {
            self.compute_hash_with_signature_latest(tx_hash)
        }
    }

    fn compute_hash_with_signature_pre_v0_11_1(&self, tx_hash: Felt) -> Felt {
        let signature_hash = match self {
            Transaction::Invoke(tx) => tx.compute_hash_signature::<Pedersen>(),
            Transaction::Declare(_)
            | Transaction::DeployAccount(_)
            | Transaction::Deploy(_)
            | Transaction::L1Handler(_) => PEDERSEN_EMPTY,
        };

        Pedersen::hash(&tx_hash, &signature_hash)
    }

    fn compute_hash_with_signature_pre_v0_13_2(&self, tx_hash: Felt) -> Felt {
        let signature_hash = match self {
            Transaction::Invoke(tx) => tx.compute_hash_signature::<Pedersen>(),
            Transaction::Declare(tx) => tx.compute_hash_signature::<Pedersen>(),
            Transaction::DeployAccount(tx) => tx.compute_hash_signature::<Pedersen>(),
            Transaction::Deploy(_) | Transaction::L1Handler(_) => PEDERSEN_EMPTY,
        };

        Pedersen::hash(&tx_hash, &signature_hash)
    }

    fn compute_hash_with_signature_pre_v0_13_4(&self, tx_hash: Felt) -> Felt {
        let signature = match self {
            Transaction::Invoke(tx) => tx.signature(),
            Transaction::Declare(tx) => tx.signature(),
            Transaction::DeployAccount(tx) => tx.signature(),
            Transaction::Deploy(_) | Transaction::L1Handler(_) => &[],
        };

        let elements = if signature.is_empty() {
            vec![tx_hash, Felt::ZERO]
        } else {
            std::iter::once(tx_hash).chain(signature.iter().copied()).collect()
        };

        Poseidon::hash_array(&elements)
    }

    fn compute_hash_with_signature_latest(&self, tx_hash: Felt) -> Felt {
        let signature = match self {
            Transaction::Invoke(tx) => tx.signature(),
            Transaction::Declare(tx) => tx.signature(),
            Transaction::DeployAccount(tx) => tx.signature(),
            Transaction::Deploy(_) | Transaction::L1Handler(_) => &[],
        };

        Poseidon::hash_array(&std::iter::once(tx_hash).chain(signature.iter().copied()).collect::<Vec<_>>())
    }
}

impl InvokeTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, legacy: bool) -> Felt {
        match self {
            InvokeTransaction::V0(tx) => tx.compute_hash(chain_id, offset_version, legacy),
            InvokeTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version),
            InvokeTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version),
        }
    }
}

impl InvokeTransactionV0 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, legacy: bool) -> Felt {
        if legacy {
            self.compute_hash_inner_legacy(chain_id)
        } else {
            self.compute_hash_inner(chain_id, offset_version)
        }
    }

    fn compute_hash_inner_legacy(&self, chain_id: Felt) -> Felt {
        let calldata_hash = Pedersen::hash_array(&self.calldata);

        Pedersen::hash_array(&[
            INVOKE_PREFIX,
            self.contract_address,
            self.entry_point_selector,
            calldata_hash,
            chain_id,
        ])
    }

    fn compute_hash_inner(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let calldata_hash = Pedersen::hash_array(&self.calldata);

        Pedersen::hash_array(&[
            INVOKE_PREFIX,
            version,
            self.contract_address,
            self.entry_point_selector,
            calldata_hash,
            self.max_fee,
            chain_id,
        ])
    }
}

impl InvokeTransactionV1 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };
        let calldata_hash = Pedersen::hash_array(&self.calldata);

        Pedersen::hash_array(&[
            INVOKE_PREFIX,
            version,
            self.sender_address,
            Felt::ZERO,
            calldata_hash,
            self.max_fee,
            chain_id,
            self.nonce,
        ])
    }
}

impl InvokeTransactionV3 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };
        let gas_hash = compute_gas_hash(self.tip, &self.resource_bounds);
        let paymaster_hash = Poseidon::hash_array(&self.paymaster_data);
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);
        let account_deployment_data_hash = Poseidon::hash_array(&self.account_deployment_data);
        let calldata_hash = Poseidon::hash_array(&self.calldata);
        let mut elements = vec![
            INVOKE_PREFIX,
            version,
            self.sender_address,
            gas_hash,
            paymaster_hash,
            chain_id,
            self.nonce,
            data_availability_modes,
            account_deployment_data_hash,
            calldata_hash,
        ];
        if let Some(proof_facts) = self.proof_facts.as_ref().filter(|proof_facts| !proof_facts.is_empty()) {
            elements.push(Poseidon::hash_array(proof_facts));
        }

        Poseidon::hash_array(&elements)
    }
}

impl L1HandlerTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, legacy: bool) -> Felt {
        if legacy {
            self.compute_hash_inner_legacy(chain_id)
        } else {
            self.compute_hash_inner(chain_id, offset_version)
        }
    }

    pub fn compute_hash_pre_v0_7(&self, chain_id: Felt) -> Felt {
        let calldata_hash = Pedersen::hash_array(&self.calldata);
        Pedersen::hash_array(&[
            INVOKE_PREFIX,
            self.contract_address,
            self.entry_point_selector,
            calldata_hash,
            chain_id,
        ])
    }

    fn compute_hash_inner_legacy(&self, chain_id: Felt) -> Felt {
        let calldata_hash = Pedersen::hash_array(&self.calldata);
        Pedersen::hash_array(&[
            L1_HANDLER_PREFIX,
            self.contract_address,
            self.entry_point_selector,
            calldata_hash,
            chain_id,
            self.nonce.into(),
        ])
    }

    fn compute_hash_inner(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let calldata_hash = Pedersen::hash_array(&self.calldata);
        Pedersen::hash_array(&[
            L1_HANDLER_PREFIX,
            version,
            self.contract_address,
            self.entry_point_selector,
            calldata_hash,
            Felt::ZERO, // Fees are set to zero on L1 Handler txs
            chain_id,
            self.nonce.into(),
        ])
    }
}

impl DeclareTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        match self {
            DeclareTransaction::V0(tx) => tx.compute_hash(chain_id, offset_version),
            DeclareTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version),
            DeclareTransaction::V2(tx) => tx.compute_hash(chain_id, offset_version),
            DeclareTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version),
        }
    }
}

impl DeclareTransactionV0 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ZERO };
        let class_or_nothing_hash = Pedersen::hash_array(&[]);

        Pedersen::hash_array(&[
            DECLARE_PREFIX,
            version,
            self.sender_address,
            Felt::ZERO,
            class_or_nothing_hash,
            self.max_fee,
            chain_id,
            self.class_hash,
        ])
    }
}

impl DeclareTransactionV1 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };
        let class_or_nothing_hash = Pedersen::hash_array(&[self.class_hash]);

        Pedersen::hash_array(&[
            DECLARE_PREFIX,
            version,
            self.sender_address,
            Felt::ZERO,
            class_or_nothing_hash,
            self.max_fee,
            chain_id,
            self.nonce,
        ])
    }
}

impl DeclareTransactionV2 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::TWO } else { Felt::TWO };
        let calldata = Pedersen::hash_array(&[self.class_hash]);

        Pedersen::hash_array(&[
            DECLARE_PREFIX,
            version,
            self.sender_address,
            Felt::ZERO,
            calldata,
            self.max_fee,
            chain_id,
            self.nonce,
            self.compiled_class_hash,
        ])
    }
}

impl DeclareTransactionV3 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };
        let gas_hash = compute_gas_hash(self.tip, &self.resource_bounds);
        let paymaster_hash = Poseidon::hash_array(&self.paymaster_data);
        let data_availability_modes =
            prepare_data_availability_modes(self.nonce_data_availability_mode, self.fee_data_availability_mode);
        let account_deployment_data_hash = Poseidon::hash_array(&self.account_deployment_data);

        Poseidon::hash_array(&[
            DECLARE_PREFIX,
            version,
            self.sender_address,
            gas_hash,
            paymaster_hash,
            chain_id,
            self.nonce,
            data_availability_modes,
            account_deployment_data_hash,
            self.class_hash,
            self.compiled_class_hash,
        ])
    }
}

impl DeployAccountTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        match self {
            DeployAccountTransaction::V1(tx) => tx.compute_hash(chain_id, offset_version),
            DeployAccountTransaction::V3(tx) => tx.compute_hash(chain_id, offset_version),
        }
    }

    pub fn calculate_contract_address(&self) -> Felt {
        match self {
            DeployAccountTransaction::V1(tx) => tx.calculate_contract_address(),
            DeployAccountTransaction::V3(tx) => tx.calculate_contract_address(),
        }
    }
}

impl DeployAccountTransactionV1 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let contract_address = self.calculate_contract_address();

        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::ONE } else { Felt::ONE };

        let mut calldata: Vec<Felt> = Vec::with_capacity(self.constructor_calldata.len() + 2);
        calldata.push(self.class_hash);
        calldata.push(self.contract_address_salt);
        calldata.extend_from_slice(&self.constructor_calldata);
        let calldata_hash = Pedersen::hash_array(calldata.as_slice());

        Pedersen::hash_array(&[
            DEPLOY_ACCOUNT_PREFIX,
            version,
            contract_address,
            Felt::ZERO,
            calldata_hash,
            self.max_fee,
            chain_id,
            self.nonce,
        ])
    }

    pub fn calculate_contract_address(&self) -> Felt {
        calculate_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Default::default(),
        )
    }
}

impl DeployAccountTransactionV3 {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool) -> Felt {
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET + Felt::THREE } else { Felt::THREE };

        let contract_address = self.calculate_contract_address();

        let gas_hash = compute_gas_hash(self.tip, &self.resource_bounds);
        let paymaster_hash = Poseidon::hash_array(&self.paymaster_data);

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
            self.nonce,
            data_availability_modes,
            constructor_calldata_hash,
            self.class_hash,
            self.contract_address_salt,
        ])
    }

    pub fn calculate_contract_address(&self) -> Felt {
        calculate_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Default::default(),
        )
    }
}

impl DeployTransaction {
    pub fn compute_hash(&self, chain_id: Felt, offset_version: bool, legacy: bool) -> Felt {
        let contract_address = self.calculate_contract_address();

        if legacy {
            compute_hash_given_contract_address_legacy(chain_id, contract_address, &self.constructor_calldata)
        } else {
            let version = if offset_version { self.version + SIMULATE_TX_VERSION_OFFSET } else { self.version };
            compute_hash_given_contract_address(version, chain_id, contract_address, &self.constructor_calldata)
        }
    }

    pub fn calculate_contract_address(&self) -> Felt {
        calculate_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Default::default(),
        )
    }
}

fn compute_hash_given_contract_address_legacy(
    chain_id: Felt,
    contract_address: Felt,
    constructor_calldata: &[Felt],
) -> Felt {
    let constructor_calldata = Pedersen::hash_array(constructor_calldata);
    let constructor = Felt::from_bytes_be(&starknet_keccak(b"constructor").to_bytes_be());

    Pedersen::hash_array(&[DEPLOY_PREFIX, contract_address, constructor, constructor_calldata, chain_id])
}

pub fn compute_hash_given_contract_address(
    version: Felt,
    chain_id: Felt,
    contract_address: Felt,
    constructor_calldata: &[Felt],
) -> Felt {
    let constructor_calldata = Pedersen::hash_array(constructor_calldata);
    let constructor = Felt::from_bytes_be(&starknet_keccak(b"constructor").to_bytes_be());

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

#[inline]
fn compute_gas_hash(tip: u64, resource_bounds: &ResourceBoundsMapping) -> Felt {
    const PREFIX_L1_GAS: &[u8; 8] = b"\0\0L1_GAS";
    const PREFIX_L2_GAS: &[u8; 8] = b"\0\0L2_GAS";
    const PREFIX_L1_DATA: &[u8; 8] = b"\0L1_DATA";

    if let Some(l1_data_gas) = &resource_bounds.l1_data_gas {
        let gas_as_felt = &[
            Felt::from(tip),
            prepare_resource_bound_value(&resource_bounds.l1_gas, PREFIX_L1_GAS),
            prepare_resource_bound_value(&resource_bounds.l2_gas, PREFIX_L2_GAS),
            prepare_resource_bound_value(l1_data_gas, PREFIX_L1_DATA),
        ];
        Poseidon::hash_array(gas_as_felt)
    } else {
        let gas_as_felt = &[
            Felt::from(tip),
            prepare_resource_bound_value(&resource_bounds.l1_gas, PREFIX_L1_GAS),
            prepare_resource_bound_value(&resource_bounds.l2_gas, PREFIX_L2_GAS),
        ];
        Poseidon::hash_array(gas_as_felt)
    }
}

// Use a mapping from execution resources to get corresponding fee bounds
// Encodes this information into 32-byte buffer then converts it into Felt
fn prepare_resource_bound_value(resource_bounds: &ResourceBounds, prefix: &[u8; 8]) -> Felt {
    let mut buffer = [0u8; 32];

    buffer[0..8].copy_from_slice(prefix);
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

const CONTRACT_ADDRESS_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f434f4e54524143545f41444452455353"); // b"STARKNET_CONTRACT_ADDRESS"
const L2_ADDRESS_UPPER_BOUND: Felt =
    Felt::from_hex_unchecked("0x7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00");

pub fn calculate_contract_address(
    salt: Felt,
    class_hash: Felt,
    constructor_calldata: &[Felt],
    deployer_address: Felt,
) -> Felt {
    let constructor_calldata_hash = Pedersen::hash_array(constructor_calldata);
    let mut address =
        Pedersen::hash_array(&[CONTRACT_ADDRESS_PREFIX, deployer_address, salt, class_hash, constructor_calldata_hash]);

    // Ensure the address is within the L2 address space
    // modulus L2_ADDRESS_UPPER_BOUND
    while address >= L2_ADDRESS_UPPER_BOUND {
        address -= L2_ADDRESS_UPPER_BOUND;
    }
    address
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{json, Value};
    use starknet_api::{core::ChainId, transaction::TransactionOptions};

    use crate::tests::{
        dummy_l1_handler, dummy_tx_declare_v0, dummy_tx_declare_v1, dummy_tx_declare_v2, dummy_tx_declare_v3,
        dummy_tx_deploy, dummy_tx_deploy_account_v1, dummy_tx_deploy_account_v3, dummy_tx_invoke_v0,
        dummy_tx_invoke_v1, dummy_tx_invoke_v3,
    };
    use crate::{ResourceBounds, MAIN_CHAIN_ID, TEST_CHAIN_ID};

    use super::*;

    const CHAIN_ID: Felt = Felt::from_hex_unchecked("0x434841494e5f4944"); // b"CHAIN_ID"

    #[test]
    fn test_compute_gas_hash() {
        let tip = 1;
        let resource_bounds = ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 2, max_price_per_unit: 3 },
            l2_gas: ResourceBounds { max_amount: 4, max_price_per_unit: 5 },
            l1_data_gas: None,
        };
        let gas_hash = compute_gas_hash(tip, &resource_bounds);
        assert_eq!(
            gas_hash,
            Felt::from_hex_unchecked("0x625cb9be49367f17655e495d674e3c04b15b6c8bfe7f2dda279252f1c1a54cd")
        );
    }

    #[test]
    fn test_prepare_data_availability_modes() {
        assert_eq!(
            prepare_data_availability_modes(DataAvailabilityMode::L1, DataAvailabilityMode::L1),
            Felt::from_hex_unchecked("0x0")
        );
        assert_eq!(
            prepare_data_availability_modes(DataAvailabilityMode::L1, DataAvailabilityMode::L2),
            Felt::from_hex_unchecked("0x100000000000000000000000000000000")
        );
        assert_eq!(
            prepare_data_availability_modes(DataAvailabilityMode::L2, DataAvailabilityMode::L1),
            Felt::from_hex_unchecked("0x10000000000000000000000000000000000000000")
        );
        assert_eq!(
            prepare_data_availability_modes(DataAvailabilityMode::L2, DataAvailabilityMode::L2),
            Felt::from_hex_unchecked("0x10000000100000000000000000000000000000000")
        );
    }

    #[test]
    fn test_compute_hash() {
        let tx: Transaction = dummy_tx_invoke_v0().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x2c6a2c329bf38089c34f4450d758f256535093b4cf29c599fee65f85ba74d0c");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_invoke_v1().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x701d05ab63a0193750f12fc7afd8014739aba9352d029af92987bc5232b2409");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_invoke_v3().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x604e0c6f351d1239d6774822b91c7e4ad61acec8ae78ad39a9f836ec9539931");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_l1_handler().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x12825a135d7176b302387a1efcc96ac55b6cb4e02fdac523c68b99f4c0cb805");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_declare_v0().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x40eff03273b068d46b1679c300da89a9ad7280301c09cafcac7ac6c96de6676");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_declare_v1().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x6bdfc70a0c9423c885ca5d7f748fee466ca2c7ea8df8becea74b171a7261e02");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_declare_v2().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x24cd4f8249b10d43ba13a60ebab01bf3c3f2c02fca8c7645cb5aba677e5d633");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_declare_v3().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x3bb257dccec1e998d332813478ad734a55b3574855b818f92f58f24a6874bfe");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_deploy().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x69ec1564562e52d5399e3faa244b9c5fdf379f0857f5ec51bd824d551f7b39b");
        assert_eq!(hash, expected_hash);

        let query_hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, true);
        let expected_query_hash =
            Felt::from_hex_unchecked("0x176fe90e0859474565355c01b086e16a1eaf7119483597b7926e52b2f71785c");
        assert_eq!(query_hash, expected_query_hash);

        let tx: Transaction = dummy_tx_deploy_account_v1().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x4b94fa32600f1e6a27ab6e3d3a42bd86cfab87842bb64a1d06a0f3daed19505");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_deploy_account_v3().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::LATEST, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x415b215091384f58219c9c9d75a31383723e4590f8480058c3902c2d92bc042");
        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn compute_hash_legacy() {
        let tx: Transaction = dummy_tx_invoke_v0().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::V0_7_0, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x4136a44fdeb38f0d879b3402dc5365785f0cf2f85fee787873ae207f0ef1171");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_l1_handler().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::V0_7_0, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x51054194b846935ec55c71a52344a0adba474fc075136b31ea8dd15b48ccfb0");
        assert_eq!(hash, expected_hash);

        let tx: Transaction = dummy_tx_deploy().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::V0_7_0, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x55714434a4cf43cdfb19af13c6f8fc0fc06f694da96566c90d3e07d32233eb5");
        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_compute_hash_pre_v0_7_l1_handler() {
        let tx: Transaction = dummy_l1_handler().into();
        let hash = tx.compute_hash(CHAIN_ID, StarknetVersion::V_0_0_0, false);
        let expected_hash =
            Felt::from_hex_unchecked("0x5bde8157ae78916bd7f86aac44d1d22e5a521d2ae7293c916f333b5d34a1602");
        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_calculate_contract_address() {
        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        let contract_address = tx.calculate_contract_address();
        let expected_contract_address =
            Felt::from_hex_unchecked("0x30b9f9d92fa7a3207cfdf60194ff8f65af01fe3ee10c0f4a1fc241882d7b413");
        assert_eq!(contract_address, expected_contract_address);

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v3().into();
        let contract_address = tx.calculate_contract_address();
        let expected_contract_address =
            Felt::from_hex_unchecked("0x734743d11641ecb3d92bafae091346fec3b2c75f7808e39f8b23d9287636e45");
        assert_eq!(contract_address, expected_contract_address,);
    }

    #[test]
    fn test_pedersen_empty() {
        assert_eq!(PEDERSEN_EMPTY, Pedersen::hash_array(&[]))
    }

    #[derive(Debug, Deserialize)]
    struct SequencerTransactionHashVector {
        block_number: u64,
        chain_id: String,
        transaction: Value,
        transaction_hash: String,
        only_query_transaction_hash: Option<String>,
    }

    fn parse_hex_u64(hex: &str) -> u64 {
        u64::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap()
    }

    fn parse_chain_id(chain_id: &str) -> Felt {
        match chain_id {
            "SN_MAIN" => MAIN_CHAIN_ID,
            "SN_SEPOLIA" => TEST_CHAIN_ID,
            other => panic!("unsupported chain id in sequencer vector: {other}"),
        }
    }

    fn normalize_transaction_json(value: &mut Value, parent_key: Option<&str>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map.iter_mut() {
                    match key.as_str() {
                        "nonce_data_availability_mode" | "fee_data_availability_mode" => {
                            let numeric = match child.as_str() {
                                Some("L1") => 0_u64,
                                Some("L2") => 1_u64,
                                Some(other) => panic!("unsupported data availability mode in test vector: {other}"),
                                None => panic!("expected string data availability mode in test vector"),
                            };
                            *child = Value::Number(numeric.into());
                        }
                        "tip" => {
                            let tip = child.as_str().map(parse_hex_u64).expect("expected hex tip in test vector");
                            *child = Value::Number(tip.into());
                        }
                        "nonce" if parent_key == Some("L1Handler") => {
                            let nonce = child
                                .as_str()
                                .map(parse_hex_u64)
                                .expect("expected hex l1 handler nonce in test vector");
                            *child = Value::Number(nonce.into());
                        }
                        _ => {}
                    }
                    normalize_transaction_json(child, Some(key));
                }
            }
            Value::Array(items) => {
                for item in items {
                    normalize_transaction_json(item, parent_key);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn parse_test_transaction(mut transaction: Value) -> Transaction {
        normalize_transaction_json(&mut transaction, None);
        serde_json::from_value(transaction).unwrap()
    }

    #[test]
    fn test_compute_hash_matches_sequencer_vectors() {
        let vectors: Vec<SequencerTransactionHashVector> =
            serde_json::from_str(include_str!("test_data/sequencer_transaction_hash_vectors.json")).unwrap();

        for vector in vectors {
            let vector_debug = format!("{vector:?}");
            let tx = parse_test_transaction(vector.transaction);
            let chain_id = parse_chain_id(&vector.chain_id);
            let starknet_version =
                StarknetVersion::try_from_mainnet_block_number(vector.block_number).unwrap_or(StarknetVersion::LATEST);

            let expected_regular_hash = Felt::from_hex(&vector.transaction_hash).unwrap();
            assert_eq!(
                tx.compute_hash(chain_id, starknet_version, false),
                expected_regular_hash,
                "regular hash mismatch for sequencer vector: {vector_debug}"
            );

            if tx.is_l1_handler() {
                continue;
            }

            let expected_query_hash =
                Felt::from_hex(vector.only_query_transaction_hash.as_deref().expect("missing only_query hash"))
                    .unwrap();
            assert_eq!(
                tx.compute_hash(chain_id, starknet_version, true),
                expected_query_hash,
                "query hash mismatch for sequencer vector: {vector_debug}"
            );
        }
    }

    #[test]
    fn test_compute_hash_matches_sepolia_replay_invoke_v3() {
        let tx = parse_test_transaction(json!({
            "Invoke": {
                "V3": {
                    "sender_address": "0x5a018f06581781371af2d639e5c905e5da998acbc53136f17e6ec99bdd77aa",
                    "calldata": [
                        "0x1",
                        "0x29b685fd5bb981ee15dd5b82e9daba0e6b4e893d64322f8308b8450914b9b04",
                        "0x7a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d",
                        "0x1",
                        "0x1"
                    ],
                    "signature": [
                        "0x2b19bdb22496139ad968f878d528fb36d2925067f6e6c981cb46bf85167fb33",
                        "0x540da32d65e745a2ecab0bb00160c437724f880da06fff1d35e3a20e86ceda9"
                    ],
                    "nonce": "0x1",
                    "resource_bounds": {
                        "L1_GAS": {
                            "max_amount": "0x10000",
                            "max_price_per_unit": "0x3a3529440000"
                        },
                        "L2_GAS": {
                            "max_amount": "0x7000000",
                            "max_price_per_unit": "0x1dcd65000"
                        },
                        "L1_DATA_GAS": {
                            "max_amount": "0x1b0",
                            "max_price_per_unit": "0x100000"
                        }
                    },
                    "tip": "0x0",
                    "paymaster_data": [],
                    "account_deployment_data": [],
                    "proof_facts": [
                        "0x50524f4f4630",
                        "0x5649525455414c5f534e4f53",
                        "0x3e98c2d7703b03a7edb73ed7f075f97f1dcbaa8f717cdf6e1a57bf058265473",
                        "0x5649525455414c5f534e4f5330",
                        "0x7af406",
                        "0x40f880f111c33c940475268be78e799921b3f30784625fede66a1fc3ea69287",
                        "0x1b9900f77ff5923183a7795fcfbb54ed76917bc1ddd4160cc77fa96e36cf8c5",
                        "0x0"
                    ],
                    "nonce_data_availability_mode": "L1",
                    "fee_data_availability_mode": "L1"
                }
            }
        }));

        let expected_hash =
            Felt::from_hex_unchecked("0x56fe231ad29fcee047c935783891e9b4b158d394b843de8aef337962eac0b9a");
        let local_hash = tx.compute_hash(TEST_CHAIN_ID, StarknetVersion::LATEST, false);
        assert_eq!(local_hash, expected_hash, "Madara hash must match the canonical Sepolia transaction hash");

        let official_tx: starknet_api::transaction::Transaction = tx.clone().try_into().unwrap();
        let official_hash = starknet_api::transaction_hash::get_transaction_hash(
            &official_tx,
            &ChainId::Sepolia,
            &TransactionOptions { only_query: false },
        )
        .unwrap();
        assert_eq!(
            Felt::from(*official_hash),
            expected_hash,
            "official starknet_api hash must match the canonical hash"
        );
    }
}
