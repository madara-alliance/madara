use mp_chain_config::StarknetVersion;
use starknet_core::utils::starknet_keccak;

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

use crate::{DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction};

use super::SIMULATE_TX_VERSION_OFFSET;

// contants for transaction prefixes
const DECLARE_PREFIX: Felt = Felt::from_hex_unchecked("0x6465636c617265"); // b"declare"
const DEPLOY_ACCOUNT_PREFIX: Felt = Felt::from_hex_unchecked("0x6465706c6f795f6163636f756e74"); // b"deploy_account"
const DEPLOY_PREFIX: Felt = Felt::from_hex_unchecked("0x6465706c6f79"); // b"deploy"
const INVOKE_PREFIX: Felt = Felt::from_hex_unchecked("0x696e766f6b65"); // b"invoke"
const L1_HANDLER_PREFIX: Felt = Felt::from_hex_unchecked("0x6c315f68616e646c6572"); // b"l1_handler"

const L1_GAS: &[u8] = b"L1_GAS";
const L2_GAS: &[u8] = b"L2_GAS";
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
            crate::Transaction::Deploy(tx) => tx.compute_hash(chain_id, legacy),
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
    /// computing the transaction commitment uses a hash value that combines
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

        let elements: Vec<Felt> = std::iter::once(tx_hash)
            .chain(signature.iter().copied())
            .collect();

        Poseidon::hash_array(&elements)
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

        Poseidon::hash_array(&[
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
        ])
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
        let class_or_nothing_hash =
            if version == Felt::ZERO { Pedersen::hash_array(&[]) } else { Pedersen::hash_array(&[self.class_hash]) };

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
        let version = if offset_version { SIMULATE_TX_VERSION_OFFSET } else { Felt::ONE };
        let class_or_nothing_hash =
            if version == Felt::ZERO { Pedersen::hash_array(&[]) } else { Pedersen::hash_array(&[self.class_hash]) };

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
    pub fn compute_hash(&self, chain_id: Felt, legacy: bool) -> Felt {
        let contract_address = self.calculate_contract_address();

        if legacy {
            compute_hash_given_contract_address_legacy(chain_id, contract_address, &self.constructor_calldata)
        } else {
            compute_hash_given_contract_address(self.version, chain_id, contract_address, &self.constructor_calldata)
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
    // Start with tip, L1_GAS, and L2_GAS
    let mut gas_elements = vec![
        Felt::from(tip),
        prepare_resource_bound_value(&resource_bounds.l1_gas, b"L1_GAS"),
        prepare_resource_bound_value(&resource_bounds.l2_gas, b"L2_GAS"),
    ];

    // Conditionally add L1 data gas if it exists
    if let Some(l1_data_gas) = &resource_bounds.l1_data_gas {
        if !l1_data_gas.is_zero() {
            gas_elements.push(prepare_resource_bound_value(l1_data_gas, b"L1_DATA"));
        }
    }

    Poseidon::hash_array(&gas_elements)
}

fn prepare_resource_bound_value(
    resource_bound: &ResourceBounds,
    name: &[u8],
) -> Felt {
    let mut buffer = [0u8; 32];

    // Split buffer: [gas_kind(8) | max_amount(8) | max_price(16)]
    let (remainder, max_price) = buffer.split_at_mut(128 / 8); // 128/8 = 16 bytes for max_price
    let (gas_kind, max_amount) = remainder.split_at_mut(64 / 8); // 64/8 = 8 bytes for gas_kind

    // Right-pad the gas kind name
    let padding = gas_kind.len() - name.len();
    gas_kind[padding..].copy_from_slice(name);

    // Copy max_amount and max_price
    max_amount.copy_from_slice(&resource_bound.max_amount.to_be_bytes());
    max_price.copy_from_slice(&resource_bound.max_price_per_unit.to_be_bytes());

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
    use crate::tests::{
        dummy_l1_handler, dummy_tx_declare_v0, dummy_tx_declare_v1, dummy_tx_declare_v2, dummy_tx_declare_v3,
        dummy_tx_deploy, dummy_tx_deploy_account_v1, dummy_tx_deploy_account_v3, dummy_tx_invoke_v0,
        dummy_tx_invoke_v1, dummy_tx_invoke_v3,
    };
    use crate::ResourceBounds;

    use super::*;

    const CHAIN_ID: Felt = Felt::from_hex_unchecked("0x434841494e5f4944"); // b"CHAIN_ID"

    #[test]
    fn test_compute_gas_hash() {
        let tip = 1;
        let resource_bounds = ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 2, max_price_per_unit: 3 },
            l2_gas: ResourceBounds { max_amount: 4, max_price_per_unit: 5 },
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
}
