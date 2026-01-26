use mp_class::{CompiledSierra, ConvertedClass, FlattenedSierraClass, SierraClassInfo, SierraConvertedClass};
use mp_convert::Felt;
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    DataAvailabilityMode, DeclareTransactionV3, DeployAccountTransactionV3, DeployTransaction, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction,
};
use serde_json;
use starknet_core::types::contract::SierraClass;
use std::sync::Arc;

pub fn devnet_account_address() -> Felt {
    Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d")
}

pub fn resource_bounds() -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: ResourceBounds { max_amount: 25_000, max_price_per_unit: 1_000_000_000 },
        l2_gas: ResourceBounds { max_amount: 50_000, max_price_per_unit: 2_000_000_000 },
        l1_data_gas: Some(ResourceBounds { max_amount: 10_000, max_price_per_unit: 500_000_000 }),
    }
}

pub fn invoke_v3(sender: Felt, nonce: Felt) -> InvokeTransactionV3 {
    InvokeTransactionV3 {
        sender_address: sender,
        calldata: vec![
            Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
            Felt::from_hex_unchecked("0x00aabb"),
            Felt::from_hex_unchecked("0x01"),
            Felt::from_hex_unchecked("0x00"),
        ]
        .into(),
        signature: vec![
            Felt::from_hex_unchecked("0x73d0a8a69f0ebf44b1c2bb2a9e85bf998883eb2008ca7b9c57b6f28dacb6dd8"),
            Felt::from_hex_unchecked("0x4a43711cd08f55ef73603f1e7b880c7f438fb68934f0823a736f9f577ab040a"),
        ]
        .into(),
        nonce,
        resource_bounds: resource_bounds(),
        tip: 1,
        paymaster_data: vec![Felt::from_hex_unchecked("0xdead")],
        account_deployment_data: vec![Felt::from_hex_unchecked("0xbeef")],
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        fee_data_availability_mode: DataAvailabilityMode::L1,
    }
}

pub fn declare_v3(sender: Felt, nonce: Felt) -> (DeclareTransactionV3, ConvertedClass) {
    let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class: FlattenedSierraClass = sierra_class.clone().flatten().unwrap().into();
    let hashes = flattened_class.compile_to_casm_with_hashes().unwrap();
    let compiled_contract_class_hash = hashes.blake_hash;
    let class_hash = sierra_class.class_hash().unwrap();
    let converted = ConvertedClass::Sierra(SierraConvertedClass {
        class_hash,
        info: SierraClassInfo {
            contract_class: Arc::new(flattened_class),
            compiled_class_hash: None,
            compiled_class_hash_v2: Some(compiled_contract_class_hash),
        },
        compiled: Arc::new(CompiledSierra::try_from(&hashes.casm_class).unwrap()),
    });

    (
        DeclareTransactionV3 {
            sender_address: sender,
            compiled_class_hash: compiled_contract_class_hash,
            signature: vec![
                Felt::from_hex_unchecked("0x1"),
                Felt::from_hex_unchecked("0x2"),
                Felt::from_hex_unchecked("0x3"),
            ]
            .into(),
            nonce,
            class_hash,
            resource_bounds: resource_bounds(),
            tip: 2,
            paymaster_data: vec![Felt::from_hex_unchecked("0x1234")],
            account_deployment_data: vec![Felt::from_hex_unchecked("0x5678")],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
        },
        converted,
    )
}

pub fn deploy_account_v3(nonce: Felt) -> DeployAccountTransactionV3 {
    DeployAccountTransactionV3 {
        signature: vec![
            Felt::from_hex_unchecked("0x73d0a8a69f0ebf44b1c2bb2a9e85bf998883eb2008ca7b9c57b6f28dacb6dd8"),
            Felt::from_hex_unchecked("0x4a43711cd08f55ef73603f1e7b880c7f438fb68934f0823a736f9f577ab040a"),
        ]
        .into(),
        nonce,
        contract_address_salt: Felt::from_hex_unchecked("0x0"),
        constructor_calldata: vec![Felt::from_hex_unchecked(
            "0x2e23f1647b018bfb3fe107e2ebd4412f0a0ed41bd60c10d842a76f8cdbbe1ba",
        )],
        class_hash: Felt::from_hex_unchecked("0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6"),
        resource_bounds: resource_bounds(),
        tip: 3,
        paymaster_data: vec![Felt::from_hex_unchecked("0xabcd")],
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        fee_data_availability_mode: DataAvailabilityMode::L1,
    }
}

pub fn deploy_tx() -> DeployTransaction {
    DeployTransaction {
        version: Felt::from_hex_unchecked("0x0"),
        contract_address_salt: Felt::from_hex_unchecked("0x1"),
        constructor_calldata: vec![Felt::from_hex_unchecked("0x1234"), Felt::from_hex_unchecked("0x5678")],
        class_hash: Felt::from_hex_unchecked("0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6"),
    }
}

pub fn l1_handler_tx() -> L1HandlerTransaction {
    L1HandlerTransaction {
        version: Felt::from_hex_unchecked("0x0"),
        nonce: 1,
        contract_address: Felt::from_hex_unchecked(
            "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
        ),
        entry_point_selector: Felt::from_hex_unchecked(
            "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad",
        ),
        calldata: vec![
            Felt::from_hex_unchecked("0x1234"),
            Felt::from_hex_unchecked("0x5678"),
            Felt::from_hex_unchecked("0x9abc"),
        ]
        .into(),
    }
}

pub fn validated_transaction(
    tx: Transaction,
    hash: Felt,
    contract_address: Felt,
    declared_class: Option<ConvertedClass>,
) -> ValidatedTransaction {
    ValidatedTransaction {
        transaction: tx,
        paid_fee_on_l1: None,
        contract_address,
        arrived_at: TxTimestamp::now(),
        declared_class,
        hash,
        charge_fee: true,
    }
}

pub fn validated_l1_handler(hash: Felt) -> ValidatedTransaction {
    let tx = Transaction::L1Handler(l1_handler_tx());
    let contract_address = match &tx {
        Transaction::L1Handler(inner) => inner.contract_address,
        _ => Felt::ZERO,
    };
    ValidatedTransaction {
        transaction: tx,
        paid_fee_on_l1: Some(123),
        contract_address,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash,
        charge_fee: false,
    }
}
