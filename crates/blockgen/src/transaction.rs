use dp_transactions::{
    DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeclareTransactionV3,
    DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction,
    InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction,
    ResourceBounds, ResourceBoundsMapping, Transaction,
};
use rand::{rngs::StdRng, Rng, SeedableRng};

use crate::felt::{random_felt, random_felts, FeltMax};

#[derive(Debug, Clone, Copy)]
pub enum TxType {
    Invoke(InvokeVersion),
    L1Handler,
    Declare(DeclareVersion),
    Deploy,
    DeployAccount(DeployAccountVersion),
}

pub struct TxTypeIterator {
    state: usize,
    variants: Vec<TxType>,
}

impl TxTypeIterator {
    pub fn new() -> Self {
        let variants = vec![
            TxType::Invoke(InvokeVersion::V0),
            TxType::Invoke(InvokeVersion::V1),
            TxType::Invoke(InvokeVersion::V3),
            TxType::L1Handler,
            TxType::Declare(DeclareVersion::V0),
            TxType::Declare(DeclareVersion::V1),
            TxType::Declare(DeclareVersion::V2),
            TxType::Declare(DeclareVersion::V3),
            TxType::Deploy,
            TxType::DeployAccount(DeployAccountVersion::V1),
            TxType::DeployAccount(DeployAccountVersion::V3),
        ];

        TxTypeIterator { state: 0, variants }
    }
}

impl Iterator for TxTypeIterator {
    type Item = TxType;

    fn next(&mut self) -> Option<Self::Item> {
        if self.variants.is_empty() {
            return None;
        }

        let item = self.variants[self.state];
        self.state = (self.state + 1) % self.variants.len();
        Some(item)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum InvokeVersion {
    V0,
    V1,
    V3,
}

#[derive(Debug, Clone, Copy)]
pub enum DeclareVersion {
    V0,
    V1,
    V2,
    V3,
}

#[derive(Debug, Clone, Copy)]
pub enum DeployAccountVersion {
    V1,
    V3,
}

pub fn dummy_transaction(seed: u64, tx_type: TxType) -> Transaction {
    match tx_type {
        TxType::Invoke(version) => Transaction::Invoke(dummy_invoke_transaction(seed, version)),
        TxType::L1Handler => Transaction::L1Handler(dummy_l1_handler_transaction(seed)),
        TxType::Declare(version) => Transaction::Declare(dummy_declare_transaction(seed, version)),
        TxType::Deploy => Transaction::Deploy(dummy_deploy_transaction(seed)),
        TxType::DeployAccount(version) => Transaction::DeployAccount(dummy_deploy_account_transaction(seed, version)),
    }
}

fn dummy_invoke_transaction(seed: u64, version: InvokeVersion) -> InvokeTransaction {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    match version {
        InvokeVersion::V0 => InvokeTransaction::V0(InvokeTransactionV0 {
            max_fee: random_felt(&mut rng, FeltMax::U128),
            signature: random_felts(&mut rng, 4),
            contract_address: random_felt(&mut rng, FeltMax::Max),
            entry_point_selector: random_felt(&mut rng, FeltMax::Max),
            calldata: random_felts(&mut rng, 4),
        }),
        InvokeVersion::V1 => InvokeTransaction::V1(InvokeTransactionV1 {
            sender_address: random_felt(&mut rng, FeltMax::Max),
            calldata: random_felts(&mut rng, 4),
            max_fee: random_felt(&mut rng, FeltMax::U128),
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
        }),
        InvokeVersion::V3 => InvokeTransaction::V3(InvokeTransactionV3 {
            sender_address: random_felt(&mut rng, FeltMax::Max),
            calldata: random_felts(&mut rng, 4),
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
            resource_bounds: random_ressource_bounds_mapping(&mut rng),
            tip: rng.gen(),
            paymaster_data: random_felts(&mut rng, 4),
            account_deployment_data: random_felts(&mut rng, 4),
            ..Default::default()
        }),
    }
}

fn dummy_l1_handler_transaction(seed: u64) -> L1HandlerTransaction {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    L1HandlerTransaction {
        version: random_felt(&mut rng, FeltMax::U64),
        nonce: rng.gen(),
        contract_address: random_felt(&mut rng, FeltMax::Max),
        entry_point_selector: random_felt(&mut rng, FeltMax::Max),
        calldata: random_felts(&mut rng, 4),
    }
}

fn dummy_declare_transaction(seed: u64, version: DeclareVersion) -> DeclareTransaction {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    match version {
        DeclareVersion::V0 => DeclareTransaction::V0(DeclareTransactionV0 {
            sender_address: random_felt(&mut rng, FeltMax::Max),
            max_fee: random_felt(&mut rng, FeltMax::U128),
            signature: random_felts(&mut rng, 4),
            class_hash: random_felt(&mut rng, FeltMax::Max),
        }),
        DeclareVersion::V1 => DeclareTransaction::V1(DeclareTransactionV1 {
            sender_address: random_felt(&mut rng, FeltMax::Max),
            max_fee: random_felt(&mut rng, FeltMax::U128),
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
            class_hash: random_felt(&mut rng, FeltMax::Max),
        }),
        DeclareVersion::V2 => DeclareTransaction::V2(DeclareTransactionV2 {
            sender_address: random_felt(&mut rng, FeltMax::Max),
            compiled_class_hash: random_felt(&mut rng, FeltMax::Max),
            max_fee: random_felt(&mut rng, FeltMax::U128),
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
            class_hash: random_felt(&mut rng, FeltMax::Max),
        }),
        DeclareVersion::V3 => DeclareTransaction::V3(DeclareTransactionV3 {
            sender_address: random_felt(&mut rng, FeltMax::Max),
            compiled_class_hash: random_felt(&mut rng, FeltMax::Max),
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
            class_hash: random_felt(&mut rng, FeltMax::Max),
            resource_bounds: random_ressource_bounds_mapping(&mut rng),
            tip: rng.gen(),
            paymaster_data: random_felts(&mut rng, 4),
            account_deployment_data: random_felts(&mut rng, 4),
            ..Default::default()
        }),
    }
}

fn dummy_deploy_transaction(seed: u64) -> DeployTransaction {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    DeployTransaction {
        version: random_felt(&mut rng, FeltMax::U64),
        contract_address_salt: random_felt(&mut rng, FeltMax::Max),
        constructor_calldata: random_felts(&mut rng, 4),
        class_hash: random_felt(&mut rng, FeltMax::Max),
    }
}

fn dummy_deploy_account_transaction(seed: u64, version: DeployAccountVersion) -> DeployAccountTransaction {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    match version {
        DeployAccountVersion::V1 => DeployAccountTransaction::V1(DeployAccountTransactionV1 {
            max_fee: random_felt(&mut rng, FeltMax::U128),
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
            contract_address_salt: random_felt(&mut rng, FeltMax::Max),
            constructor_calldata: random_felts(&mut rng, 4),
            class_hash: random_felt(&mut rng, FeltMax::Max),
        }),
        DeployAccountVersion::V3 => DeployAccountTransaction::V3(DeployAccountTransactionV3 {
            signature: random_felts(&mut rng, 4),
            nonce: random_felt(&mut rng, FeltMax::U64),
            contract_address_salt: random_felt(&mut rng, FeltMax::Max),
            constructor_calldata: random_felts(&mut rng, 4),
            class_hash: random_felt(&mut rng, FeltMax::Max),
            resource_bounds: random_ressource_bounds_mapping(&mut rng),
            tip: rng.gen(),
            paymaster_data: random_felts(&mut rng, 4),
            ..Default::default()
        }),
    }
}

fn random_ressource_bounds_mapping(rng: &mut StdRng) -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: ResourceBounds { max_amount: rng.gen(), max_price_per_unit: rng.gen() },
        l2_gas: ResourceBounds { max_amount: rng.gen(), max_price_per_unit: rng.gen() },
    }
}

#[cfg(test)]
mod tests {
    use starknet_core::types::Felt;

    use super::*;

    #[test]
    fn test_random_transaction() {
        let mut hashes = Vec::new();
        let mut tx_type_iter = TxTypeIterator::new();

        let start = std::time::Instant::now();
        for i in 0..1_000 {
            let tx_type = tx_type_iter.next().unwrap();

            let tx = dummy_transaction(i, tx_type);
            let hash = tx.compute_hash(Felt::ZERO, false, false);
            hashes.push(hash);
        }
        println!("1_000 transactions generated with hash in {:?}", start.elapsed());

        let checksum = hashes.iter().fold([0u8; 32], |acc, hash| {
            let mut acc = acc;
            let hash = hash.to_bytes_be();
            for i in 0..32 {
                acc[i] ^= hash[i];
            }
            acc
        });
        println!("Checksum: {:?}", checksum);
    }
}
