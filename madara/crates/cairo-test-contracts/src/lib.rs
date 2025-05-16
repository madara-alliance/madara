//! This crate is used to build the cairo test contracts for tests. Contracts that are not used in
//! tests need to be put in the `cairo-artifacts` folder at the root of the project`.

pub const TEST_CONTRACT_SIERRA: &[u8] =
    include_bytes!("../../../../build_artifacts/js_tests/madara_contracts_TestContract.contract_class.json");
pub const APPCHAIN_CONTRACT_SIERRA: &[u8] =
    include_bytes!("../../../../build_artifacts/js_tests/madara_contracts_StateUpdateContract.contract_class.json");
pub const MESSAGING_CONTRACT_SIERRA: &[u8] =
    include_bytes!("../../../../build_artifacts/js_tests/madara_contracts_MessagingContract.contract_class.json");
